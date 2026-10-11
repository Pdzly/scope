//! pipx main environments remain separate from their injected distributions.
use super::{fingerprint, package, query, validate_name, ToolInstallation, UpdateRecipe};
use crate::domain::package::{InstalledPackage, PackageSource};
use anyhow::{Context, Result};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

pub(super) async fn inventory(
    manager: PathBuf,
    context: Option<&ToolInstallation>,
) -> Result<Vec<InstalledPackage>> {
    let (env, output) = if let Some(context) = context {
        (
            context.env.clone(),
            super::query_installation(context, &manager, &["list", "--json"]).await?,
        )
    } else {
        let mut env = BTreeMap::new();
        for key in ["PIPX_HOME", "PIPX_BIN_DIR", "PIPX_MAN_DIR"] {
            env.insert(
                key.into(),
                query(&manager, &["environment", "--value", key])
                    .await?
                    .trim()
                    .into(),
            );
        }
        (env, query(&manager, &["list", "--json"]).await?)
    };
    parse(&output, manager, env)
}

pub(super) fn parse(
    output: &str,
    manager: PathBuf,
    env: BTreeMap<String, String>,
) -> Result<Vec<InstalledPackage>> {
    let root = PathBuf::from(env.get("PIPX_HOME").context("pipx home missing")?);
    let bin = PathBuf::from(
        env.get("PIPX_BIN_DIR")
            .context("pipx bin directory missing")?,
    );
    let inventory: Value = serde_json::from_str(output).context("Invalid pipx inventory")?;
    let venvs = inventory
        .get("venvs")
        .and_then(Value::as_object)
        .context("pipx inventory has no environments")?;
    let mut packages = Vec::new();
    for (id, venv) in venvs {
        validate_name(id)?;
        let metadata = venv.get("metadata").context("pipx metadata missing")?;
        let main = metadata
            .get("main_package")
            .context("pipx main package missing")?;
        let name = main
            .get("package")
            .and_then(Value::as_str)
            .context("pipx package missing")?;
        validate_name(name)?;
        let version = main
            .get("package_version")
            .and_then(Value::as_str)
            .context("pipx version missing")?;
        let spec = main
            .get("package_or_url")
            .and_then(Value::as_str)
            .context("pipx source missing")?;
        let pinned = main.get("pinned").and_then(Value::as_bool).unwrap_or(false);
        let pip_args = strings(main.get("pip_args"))?;
        let mut executables = Vec::new();
        for pkg in std::iter::once(main).chain(
            metadata
                .get("injected_packages")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|m| m.values()),
        ) {
            if pkg.get("include_apps").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let suffix = main.get("suffix").and_then(Value::as_str).unwrap_or("");
            for app in strings(pkg.get("apps"))? {
                let executable = format!("{app}{suffix}");
                validate_name(&executable)?;
                executables.push(bin.join(executable));
            }
            if pkg.get("include_dependencies").and_then(Value::as_bool) == Some(true) {
                if let Some(apps) = pkg.get("apps_of_dependencies").and_then(Value::as_object) {
                    for list in apps.values() {
                        for app in strings(Some(list))? {
                            let executable = format!("{app}{suffix}");
                            validate_name(&executable)?;
                            executables.push(bin.join(executable));
                        }
                    }
                }
            }
        }
        executables.sort();
        executables.dedup();
        let injected = metadata
            .get("injected_packages")
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let mut notes = vec!["Uninstall removes this environment and its exposed commands. Personal data and shared Python are kept.".into(),
            "pipx upgrades may remove original version constraints and maintain shared packaging libraries.".into()];
        if !injected.is_empty() {
            notes.push(format!("Injected packages removed with this environment: {}. Updates retain them without requesting their upgrade.", injected.join(", ")));
        }
        let installation = ToolInstallation {
            source: PackageSource::Pipx,
            manager: manager.clone(),
            root: root.clone(),
            environment: Some(root.join("venvs").join(id)),
            executables,
            env: env.clone(),
            protection_reason: None,
            receipt_hash: fingerprint(serde_json::to_string(metadata)?.as_bytes()),
            update: UpdateRecipe::Pipx {
                package: name.into(),
                pip_args,
                pinned,
                index_source: !spec.contains(['/', ':', '@']),
            },
            notes,
        };
        packages.push(package(id, version, installation)?);
    }
    Ok(packages)
}

fn strings(value: Option<&Value>) -> Result<Vec<String>> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .context("Invalid pipx string list")
        })
        .collect()
}
