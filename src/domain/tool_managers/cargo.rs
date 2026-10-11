//! Reconstructable Cargo receipts; Cargo's command list is not an inventory.
use super::{fingerprint, home, package, read, validate_name, ToolInstallation, UpdateRecipe};
use crate::domain::package::{InstalledPackage, PackageSource};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Deserialize)]
struct Receipt {
    installs: BTreeMap<String, CargoInstall>,
}
#[derive(Deserialize, serde::Serialize)]
struct CargoInstall {
    bins: Vec<String>,
    features: Vec<String>,
    all_features: bool,
    no_default_features: bool,
    profile: String,
    target: String,
    version_req: Option<String>,
}

pub(super) async fn inventory(
    manager: PathBuf,
    context: Option<&ToolInstallation>,
) -> Result<Vec<InstalledPackage>> {
    if let Some(context) = context {
        return read_inventory(manager, context.root.clone(), context.env.clone()).await;
    }
    let user_home = home()?;
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or(user_home.join(".cargo"));
    let cargo_home = if cargo_home.is_absolute() {
        cargo_home
    } else {
        user_home.join(cargo_home)
    };
    let root = if let Some(root) = std::env::var_os("CARGO_INSTALL_ROOT") {
        let root = PathBuf::from(root);
        if root.is_absolute() {
            root
        } else {
            user_home.join(root)
        }
    } else {
        let mut directories = user_home
            .ancestors()
            .map(|path| path.join(".cargo"))
            .collect::<Vec<_>>();
        directories.push(cargo_home.clone());
        config_root(&directories)
            .await?
            .unwrap_or(cargo_home.clone())
    };
    let mut env = BTreeMap::new();
    env.insert(
        "CARGO_HOME".into(),
        cargo_home.to_string_lossy().into_owned(),
    );
    env.insert(
        "CARGO_INSTALL_ROOT".into(),
        root.to_string_lossy().into_owned(),
    );
    read_inventory(manager, root, env).await
}

pub(super) fn parse(
    raw: &str,
    manager: PathBuf,
    root: PathBuf,
    env: BTreeMap<String, String>,
) -> Result<Vec<InstalledPackage>> {
    let receipt: Receipt =
        serde_json::from_str(raw).context("Invalid Cargo installation receipt")?;
    let mut packages = Vec::new();
    for (identity, record) in receipt.installs {
        let (prefix, origin) = identity
            .split_once(" (")
            .context("Invalid Cargo package identity")?;
        let (id, version) = prefix.split_once(' ').context("Invalid Cargo version")?;
        validate_name(id)?;
        let origin = origin.strip_suffix(')').context("Invalid Cargo source")?;
        let mut options = Vec::new();
        let mut executables = Vec::new();
        for bin in &record.bins {
            validate_name(bin)?;
            executables.push(root.join("bin").join(bin));
            options.extend(["--bin".into(), bin.clone()]);
        }
        if record.bins.is_empty() {
            bail!("Cargo receipt has no installed binaries");
        }
        for feature in &record.features {
            if feature.is_empty()
                || feature.starts_with('-')
                || feature.contains(char::is_whitespace)
            {
                bail!("Invalid Cargo feature");
            }
        }
        if !record.features.is_empty() {
            options.extend(["--features".into(), record.features.join(",")]);
        }
        if record.all_features {
            options.push("--all-features".into());
        }
        if record.no_default_features {
            options.push("--no-default-features".into());
        }
        validate_name(&record.profile)?;
        validate_name(&record.target)?;
        options.extend([
            "--profile".into(),
            record.profile.clone(),
            "--target".into(),
            record.target.clone(),
        ]);
        let installation = ToolInstallation { source: PackageSource::Cargo, manager: manager.clone(), root: root.clone(), environment: None,
            executables, env: env.clone(), protection_reason: None, receipt_hash: fingerprint(format!("{identity}{}", serde_json::to_string(&record)?).as_bytes()),
            update: UpdateRecipe::Cargo { origin: origin.into(), options, version_req: record.version_req },
            notes: vec!["Uninstall removes only this crate's tracked binaries. Shared registry caches, toolchains, and personal data are kept.".into(),
                "Updates compile the original source and may run build scripts. Installed features, binaries, profile, and target are retained.".into()] };
        let mut pkg = package(id, version, installation)?;
        if origin.starts_with("git+") {
            if let Some((_, revision)) = origin.rsplit_once('#') {
                pkg.display_version = Some(version.into());
                pkg.version = format!("{version}#{revision}");
            }
        }
        packages.push(pkg);
    }
    Ok(packages)
}

async fn read_inventory(
    manager: PathBuf,
    root: PathBuf,
    env: BTreeMap<String, String>,
) -> Result<Vec<InstalledPackage>> {
    let path = root.join(".crates2.json");
    if !path.exists() {
        if root.join(".crates.toml").exists() {
            bail!("Cargo build settings are missing. Reinstall tracked tools with current Cargo before managing them here.");
        }
        return Ok(Vec::new());
    }
    let packages = parse(&read(&path).await?, manager, root.clone(), env)?;
    #[derive(Deserialize)]
    struct Ownership {
        v1: BTreeMap<String, Vec<String>>,
    }
    let ownership: Ownership = toml::from_str(&read(&root.join(".crates.toml")).await?)?;
    let mut bins = std::collections::HashSet::new();
    for pkg in &packages {
        let installation = pkg
            .tool_installation
            .as_ref()
            .context("Cargo receipt missing")?;
        let UpdateRecipe::Cargo { origin, .. } = &installation.update else {
            bail!("Cargo source missing");
        };
        let identity = format!("{} {} ({origin})", pkg.package_id, pkg.version_label());
        let mut tracked = ownership
            .v1
            .get(&identity)
            .context(
                "Cargo ownership records disagree. Rescan or repair the installation with Cargo.",
            )?
            .clone();
        let mut expected = installation
            .executables
            .iter()
            .map(|path| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        tracked.sort();
        expected.sort();
        if tracked != expected || expected.iter().any(|bin| !bins.insert(bin.clone())) {
            bail!("Cargo binary ownership is ambiguous");
        }
    }
    if ownership.v1.len() != packages.len() {
        bail!("Cargo ownership records are incomplete");
    }
    Ok(packages)
}

pub(super) async fn config_root(directories: &[PathBuf]) -> Result<Option<PathBuf>> {
    for directory in directories {
        // Cargo prefers legacy config when both names exist.
        let path = ["config", "config.toml"]
            .iter()
            .map(|name| directory.join(name))
            .find(|path| path.exists());
        if let Some(path) = path {
            let config: toml::Value = toml::from_str(&read(&path).await?)?;
            if let Some(root) = config
                .get("install")
                .and_then(|value| value.get("root"))
                .and_then(toml::Value::as_str)
            {
                let root = PathBuf::from(root);
                return Ok(Some(if root.is_absolute() {
                    root
                } else {
                    directory
                        .parent()
                        .context("Cargo config parent missing")?
                        .join(root)
                }));
            }
        }
    }
    Ok(None)
}
