//! One row per global package, using native npm inventory or Bun's global manifest.
use super::javascript_config::{discover, GlobalContext};
use super::{
    fingerprint, home, package, read, validate_name, validate_package_name, ToolInstallation,
    UpdateRecipe,
};
use crate::domain::{
    package::{InstalledPackage, PackageSource},
    system,
};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) async fn inventory(
    source: PackageSource,
    manager: PathBuf,
    recorded: Option<&ToolInstallation>,
) -> Result<Vec<InstalledPackage>> {
    let context = if let Some(recorded) = recorded {
        let UpdateRecipe::Javascript { bin, node, .. } = &recorded.update else {
            bail!("Global tool metadata missing");
        };
        GlobalContext {
            root: recorded.root.clone(),
            bin: bin.clone(),
            node: node.clone(),
            env: recorded.env.clone(),
            config_hash: discover(source, &manager).await?.config_hash,
        }
    } else {
        discover(source, &manager).await?
    };
    let modules = modules(source, &context.root);
    if !modules.try_exists()? {
        return Ok(Vec::new());
    }
    let receipt = if source == PackageSource::Npm {
        let prefix = context.root.to_str().context("npm prefix is not UTF-8")?;
        let env = context
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>();
        let output = system::capture_output_context(
            manager.to_str().context("Manager path is not UTF-8")?,
            &[
                "ls",
                "--global",
                "--prefix",
                prefix,
                "--depth=0",
                "--json",
                "--long",
            ],
            &env,
            Some(&home()?),
            Duration::from_secs(30),
        )
        .await?;
        let value: Value =
            serde_json::from_str(&output.stdout).context("Invalid npm global inventory")?;
        if !output.success || value.get("error").is_some() {
            bail!("npm global inventory could not be verified");
        }
        value
    } else {
        let manifest = context.root.join("package.json");
        if !manifest.try_exists()? {
            return Ok(Vec::new());
        }
        serde_json::from_str(&read(&manifest).await?).context("Invalid Bun global manifest")?
    };
    read_inventory(source, manager, context, &receipt).await
}

pub(super) fn modules(source: PackageSource, root: &Path) -> PathBuf {
    root.join(if source == PackageSource::Npm {
        "lib/node_modules"
    } else {
        "node_modules"
    })
}

pub(super) async fn read_inventory(
    source: PackageSource,
    manager: PathBuf,
    context: GlobalContext,
    receipt: &Value,
) -> Result<Vec<InstalledPackage>> {
    let Some(entries) = receipt.get("dependencies").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut packages = Vec::new();
    let mut shared_receipt = context.config_hash.clone();
    // A change elsewhere in the shared tree invalidates previously reviewed removal effects.
    // npm emits package objects in completion order, which is not installation identity.
    let mut canonical = receipt.clone();
    canonical.sort_all_objects();
    shared_receipt.push_str(&serde_json::to_string(&canonical)?);
    shared_receipt.push_str(&super::javascript_safety::config_receipt(&context.root).await?);
    for file in [
        "bun.lock",
        "bun.lockb",
        "package-lock.json",
        "node_modules/.package-lock.json",
    ] {
        let path = context.root.join(file);
        if path.try_exists()? {
            // Binary Bun lockfiles are hashed as bytes, with the same bound as textual receipts.
            use tokio::io::AsyncReadExt;
            let mut bytes = Vec::new();
            tokio::fs::File::open(path)
                .await?
                .take(2 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .await?;
            if bytes.len() > 2 * 1024 * 1024 {
                bail!("Global lockfile exceeds the metadata limit");
            }
            shared_receipt.push_str(&fingerprint(&bytes));
        }
    }
    for (id, entry) in entries {
        validate_package_name(source, id)?;
        if entry.get("extraneous").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let environment = modules(source, &context.root).join(id);
        let raw = read(&environment.join("package.json")).await?;
        let manifest: Value =
            serde_json::from_str(&raw).context("Invalid installed JavaScript package")?;
        let package_name = manifest
            .get("name")
            .and_then(Value::as_str)
            .context("Installed JavaScript package name missing")?;
        validate_package_name(source, package_name)?;
        let version = manifest
            .get("version")
            .and_then(Value::as_str)
            .context("Installed JavaScript version missing")?;
        semver::Version::parse(version).context("Invalid installed JavaScript version")?;
        if source == PackageSource::Npm
            && entry.get("version").and_then(Value::as_str) != Some(version)
        {
            bail!("npm inventory disagrees with the installed version");
        }
        let requirement = if source == PackageSource::Bun {
            entry
                .as_str()
                .context("Invalid Bun dependency requirement")?
                .to_string()
        } else {
            "*".into()
        };
        let origin = entry
            .get("resolved")
            .and_then(Value::as_str)
            .or_else(|| manifest.get("_resolved").and_then(Value::as_str))
            .unwrap_or("");
        let registry_source = package_name == id
            && registry_requirement(&requirement)
            && !origin.starts_with("git+")
            && !origin.starts_with("file:")
            && !environment.join(".git").exists()
            && !environment.symlink_metadata()?.file_type().is_symlink();
        let commands = commands(package_name, &manifest)?;
        let executables = commands.keys().map(|name| context.bin.join(name)).collect();
        let mut identity = shared_receipt.clone();
        identity.push_str(&raw);
        let installation = ToolInstallation {
            source, manager: manager.clone(), root: context.root.clone(), environment: Some(environment), executables,
            env: context.env.clone(), receipt_hash: fingerprint(identity.as_bytes()), protection_reason: None,
            update: UpdateRecipe::Javascript { package_name: package_name.into(), bin: context.bin.clone(), node: context.node.clone(), requirement, registry_source },
            notes: vec![
                "Global installation only; project dependencies are not managed.".into(),
                "The manager may reconcile shared dependencies. Shared caches, runtimes, and personal data are kept.".into(),
                if source == PackageSource::Npm { "npm global updates use the configured registry; global installs do not retain an original version range.".into() }
                else { "Bun updates preserve the global manifest's version constraints and script trust settings.".into() },
            ],
        };
        let mut pkg = package(id, version, installation)?;
        pkg.description = manifest
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned);
        packages.push(pkg);
    }
    Ok(packages)
}

pub(super) fn commands(
    id: &str,
    manifest: &Value,
) -> Result<std::collections::BTreeMap<String, PathBuf>> {
    let mut commands = std::collections::BTreeMap::new();
    let Some(bin) = manifest.get("bin") else {
        return Ok(commands);
    };
    let values = if let Some(path) = bin.as_str() {
        vec![(
            id.rsplit('/')
                .next()
                .context("Missing package name")?
                .to_string(),
            path.to_string(),
        )]
    } else {
        bin.as_object()
            .context("Invalid JavaScript bin metadata")?
            .iter()
            .map(|(name, path)| {
                Ok((
                    name.clone(),
                    path.as_str()
                        .context("Invalid JavaScript command target")?
                        .to_string(),
                ))
            })
            .collect::<Result<Vec<_>>>()?
    };
    for (name, target) in values {
        validate_name(&name)?;
        let path = PathBuf::from(target);
        if path.is_absolute()
            || path.components().any(|part| {
                !matches!(
                    part,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )
            })
        {
            bail!("JavaScript command escapes its package directory");
        }
        commands.insert(name, path);
    }
    Ok(commands)
}

pub(super) fn registry_requirement(requirement: &str) -> bool {
    !requirement.is_empty()
        && requirement.len() <= 256
        && !requirement.contains(['/', ':', '@', '\n', '\r'])
        && !requirement.starts_with('-')
}

#[cfg(test)]
#[path = "javascript_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "javascript_live_tests.rs"]
mod live_tests;
