//! Explicit-scope deployment lookup and current-user app data, never runtimes.

use anyhow::{bail, Result};
use std::path::PathBuf;
use std::time::Duration;

use super::{
    targets::{Discovery, Target},
    DetailEffect,
};
use crate::domain::{
    package::{InstallScope, InstalledPackage},
    system::capture_stdout,
};

fn info_args(pkg: &InstalledPackage) -> Result<[&str; 4]> {
    let scope = match pkg.install_scope {
        Some(InstallScope::User) => "--user",
        Some(InstallScope::System) => "--system",
        None => bail!("Flatpak installation scope is missing. Rescan to inspect this app."),
    };
    let id = pkg.package_id.as_str();
    if id.is_empty()
        || !id.as_bytes()[0].is_ascii_alphabetic()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        bail!("Invalid Flatpak app id");
    }
    Ok(["info", scope, "--show-location", id])
}

pub(super) async fn discover(pkg: &InstalledPackage) -> Result<Discovery> {
    let output = capture_stdout("flatpak", &info_args(pkg)?, Duration::from_secs(5))
        .await
        .map_err(|_| anyhow::anyhow!("Flatpak deployment location could not be read"))?;
    let location = PathBuf::from(output.trim());
    if !location.is_absolute() || output.lines().count() != 1 {
        bail!("Flatpak returned an invalid deployment location");
    }
    let mut discovery = Discovery::default();
    discovery.add(
        "Application",
        location.display().to_string(),
        DetailEffect::Removed,
        Target::Directory(location),
    );
    if let Some(home) = std::env::var_os("HOME") {
        add_data(
            &mut discovery,
            PathBuf::from(home).join(".var/app").join(&pkg.package_id),
        );
    } else {
        discovery.unknown("User data", "Home directory unavailable");
    }
    Ok(discovery)
}

fn add_data(discovery: &mut Discovery, root: PathBuf) {
    // Normal flatpak uninstall does not use --delete-data. One parent row
    // avoids double-counting cache/configuration alongside the same data tree.
    discovery.add(
        "User data",
        root.display().to_string(),
        DetailEffect::Kept,
        Target::Directory(root),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::package::PackageSource;

    #[test]
    fn lookup_uses_recorded_scope_and_rejects_missing_scope() {
        for (scope, flag) in [
            (InstallScope::User, "--user"),
            (InstallScope::System, "--system"),
        ] {
            let pkg =
                InstalledPackage::new_scoped(PackageSource::Flatpak, "org.example.App", scope);
            assert_eq!(
                info_args(&pkg).unwrap(),
                ["info", flag, "--show-location", "org.example.App"]
            );
        }
        assert!(info_args(&InstalledPackage::new(
            PackageSource::Flatpak,
            "org.example.App"
        ))
        .is_err());
    }

    #[test]
    fn user_data_is_retained_without_duplicate_subdirectory_rows() {
        let mut discovery = Discovery::default();
        add_data(
            &mut discovery,
            PathBuf::from("/home/test/.var/app/org.example.App"),
        );
        assert_eq!(discovery.details.items.len(), 1);
        assert_eq!(discovery.details.items[0].effect, DetailEffect::Kept);
    }
}
