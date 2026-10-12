//! Verify global package paths and every exposed command before measurement or mutation.
use super::{javascript, javascript_config, read, ToolInstallation, UpdateRecipe};
use crate::domain::package::PackageSource;
use anyhow::{bail, Context, Result};
use std::{collections::BTreeMap, path::PathBuf};

pub(super) fn validate(installation: &ToolInstallation) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let UpdateRecipe::Javascript {
        package_name,
        bin,
        node,
        requirement,
        ..
    } = &installation.update
    else {
        bail!("Global JavaScript installation metadata missing");
    };
    let expected = match installation.source {
        PackageSource::Npm => {
            let runtime = node.as_ref().context("Recorded Node runtime missing")?;
            if !runtime.is_absolute() || !runtime.is_file() {
                bail!("Recorded Node runtime is unavailable");
            }
            if bin != &installation.root.join("bin") {
                bail!("npm commands must use the recorded prefix");
            }
            BTreeMap::from([
                (
                    "NPM_CONFIG_PREFIX".into(),
                    installation.root.display().to_string(),
                ),
                ("PATH".into(), javascript_config::node_path(runtime)?),
            ])
        }
        PackageSource::Bun => {
            if node.is_some() {
                bail!("Unexpected Node runtime in Bun metadata");
            }
            BTreeMap::from([
                (
                    "BUN_INSTALL_GLOBAL_DIR".into(),
                    installation.root.display().to_string(),
                ),
                ("BUN_INSTALL_BIN".into(), bin.display().to_string()),
            ])
        }
        _ => bail!("Not a global JavaScript installation"),
    };
    if installation.env != expected || requirement.is_empty() {
        bail!("Recorded global tool settings are inconsistent");
    }
    if installation.root.canonicalize()? != installation.root || bin.canonicalize()? != *bin {
        bail!("Recorded global directory now resolves to another location");
    }
    let environment = installation
        .environment
        .as_ref()
        .context("Global package directory missing")?;
    let modules = javascript::modules(installation.source, &installation.root);
    let id = environment
        .strip_prefix(&modules)
        .context("Package is outside its global modules directory")?
        .to_str()
        .context("Package name is not UTF-8")?;
    super::validate_package_name(installation.source, id)?;
    if environment.canonicalize()? != modules.canonicalize()?.join(id) {
        bail!("Global package directory has an indirect symbolic link");
    }
    use std::io::Read;
    let mut raw = String::new();
    std::fs::File::open(environment.join("package.json"))?
        .take(2 * 1024 * 1024 + 1)
        .read_to_string(&mut raw)?;
    if raw.len() > 2 * 1024 * 1024 {
        bail!("Package manifest exceeds the metadata limit");
    }
    let manifest: serde_json::Value = serde_json::from_str(&raw)?;
    super::validate_package_name(installation.source, package_name)?;
    let protection = crate::domain::safety::check_package(installation.source, package_name);
    if protection.protected {
        bail!("{}", protection.reason.unwrap_or_default());
    }
    if manifest.get("name").and_then(serde_json::Value::as_str) != Some(package_name) {
        bail!("Global package identity changed");
    }
    let commands = javascript::commands(package_name, &manifest)?;
    let mut expected_commands = Vec::new();
    for (name, target) in commands {
        let executable = bin.join(name);
        let target = environment
            .join(target)
            .canonicalize()
            .context("Global command target is missing")?;
        if !target.starts_with(environment.canonicalize()?)
            || executable.canonicalize().ok().as_ref() != Some(&target)
        {
            bail!("Global command belongs to another installation or is missing");
        }
        expected_commands.push(executable);
    }
    let mut recorded = installation.executables.clone();
    expected_commands.sort();
    recorded.sort();
    if expected_commands != recorded {
        bail!("Recorded global commands disagree with the package manifest");
    }
    let home = super::home()?.canonicalize()?;
    let resolved = bin.canonicalize()?;
    if !bin.is_absolute()
        || !resolved.starts_with(&home)
        || std::fs::metadata(&resolved)?.uid() != std::fs::metadata(&home)?.uid()
    {
        bail!("Global command directory is not user-owned");
    }
    Ok(())
}

pub(super) async fn verify_context(installation: &ToolInstallation) -> Result<()> {
    let live = javascript_config::discover(installation.source, &installation.manager).await?;
    let UpdateRecipe::Javascript { bin, node, .. } = &installation.update else {
        bail!("Global tool metadata missing");
    };
    if live.root != installation.root
        || live.bin != *bin
        || live.node != *node
        || live.env != installation.env
    {
        bail!("The global installation root or runtime changed. Rescan and preview again.");
    }
    Ok(())
}

pub(super) async fn config_receipt(root: &std::path::Path) -> Result<String> {
    let mut raw = String::new();
    for name in [".npmrc", "etc/npmrc", "bunfig.toml"] {
        let path: PathBuf = root.join(name);
        if path.try_exists()? {
            raw.push_str(&read(&path).await?);
        }
    }
    Ok(super::fingerprint(raw.as_bytes()))
}
