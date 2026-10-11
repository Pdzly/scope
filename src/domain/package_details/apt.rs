//! dpkg-owned files and conffiles, without walking shared system directories.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{
    targets::{Discovery, Target},
    DetailEffect,
};
use crate::domain::{package::InstalledPackage, system::capture_stdout};
use anyhow::{bail, Result};

pub(super) async fn discover(pkg: &InstalledPackage) -> Result<Discovery> {
    let name = &pkg.package_id;
    if name.is_empty()
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+.-:".contains(&b))
    {
        bail!("Invalid APT package name");
    }
    let file_args = ["--listfiles", name];
    let config_args = ["--show", "--showformat=${Conffiles}", name];
    let (files, configs) = tokio::join!(
        capture_stdout("dpkg-query", &file_args, Duration::from_secs(5)),
        capture_stdout("dpkg-query", &config_args, Duration::from_secs(5)),
    );
    let files = files.map_err(|_| anyhow::anyhow!("Package-owned files could not be read"))?;
    let configs = configs
        .map_err(|_| anyhow::anyhow!("Package configuration ownership could not be read"))?;
    if files.contains("[scope] output truncated") || configs.contains("[scope] output truncated") {
        bail!("Package metadata exceeds the inspection limit");
    }
    tokio::task::spawn_blocking(move || group_files(&files, &configs)).await?
}

fn conffiles(output: &str) -> Result<HashSet<PathBuf>> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let line = line.trim().strip_suffix(" obsolete").unwrap_or(line.trim());
            let Some((path, checksum)) = line.rsplit_once(' ') else {
                bail!("Package configuration ownership could not be verified");
            };
            if checksum.len() != 32
                || !checksum.bytes().all(|b| b.is_ascii_hexdigit())
                || !path.starts_with('/')
            {
                bail!("Package configuration ownership could not be verified");
            }
            Ok(PathBuf::from(path))
        })
        .collect()
}

fn group_files(files: &str, configs: &str) -> Result<Discovery> {
    let configs = conffiles(configs)?;
    let mut groups: BTreeMap<(String, String, bool), Vec<PathBuf>> = BTreeMap::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    for (count, path) in files.lines().map(PathBuf::from).enumerate() {
        if count >= 20_000 || Instant::now() >= deadline {
            bail!("Package file list exceeds the inspection limit");
        }
        if !path.is_absolute() || path.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let kept = configs.contains(&path);
        let (label, root) = group(&path, kept);
        groups
            .entry((label.into(), root, kept))
            .or_default()
            .push(path);
    }
    if groups.is_empty() {
        bail!("No package-owned files could be identified");
    }
    let mut discovery = Discovery::default();
    for ((label, root, kept), paths) in groups {
        let count = paths.len();
        let location = format!(
            "{root} · {count} package file{}",
            if count == 1 { "" } else { "s" }
        );
        discovery.add(
            &label,
            location,
            if kept {
                DetailEffect::Kept
            } else {
                DetailEffect::Removed
            },
            Target::OwnedFiles(paths),
        );
    }
    discovery.unknown("User data", "APT does not identify app-created user files");
    Ok(discovery)
}

fn group(path: &Path, kept: bool) -> (&'static str, String) {
    let (label, depth) = if kept {
        ("Configuration", 2)
    } else if path.starts_with("/etc") {
        ("System files", 3)
    } else if path.starts_with("/usr/share/doc") {
        ("Documentation", 5)
    } else if path.starts_with("/usr/share/applications") {
        ("Launchers", 4)
    } else if path.starts_with("/usr/share/icons") || path.starts_with("/usr/share/pixmaps") {
        ("Icons", 4)
    } else if path.starts_with("/usr/share/bash-completion") || path.starts_with("/usr/share/zsh") {
        ("Shell integration", 4)
    } else if [
        "/usr/share/appdata",
        "/usr/share/metainfo",
        "/usr/share/mime",
    ]
    .iter()
    .any(|root| path.starts_with(root))
    {
        ("App metadata", 4)
    } else if path.starts_with("/usr/share") || path.starts_with("/usr/lib") {
        ("Application", 4)
    } else {
        ("Application", 3)
    };
    (
        label,
        path.components()
            .take(depth)
            .collect::<PathBuf>()
            .display()
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configurations_are_retained_and_paths_with_spaces_are_preserved() {
        let configs = " /etc/test app.conf 0123456789abcdef0123456789abcdef obsolete\n";
        assert!(conffiles(configs)
            .unwrap()
            .contains(Path::new("/etc/test app.conf")));
        let result = group_files("/usr/share/example/app\n/etc/test app.conf\n", configs).unwrap();
        assert!(result
            .details
            .items
            .iter()
            .any(|i| i.label == "Configuration" && i.effect == DetailEffect::Kept));
        assert!(result
            .details
            .items
            .iter()
            .any(|i| i.label == "Application" && i.effect == DetailEffect::Removed));
    }

    #[test]
    fn malformed_configuration_metadata_never_becomes_a_removal_claim() {
        assert!(group_files("/etc/test.conf\n", " /etc/test.conf unknown\n").is_err());
        assert!(conffiles("\n").unwrap().is_empty());
    }
}
