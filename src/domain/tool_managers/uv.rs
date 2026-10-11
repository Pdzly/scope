//! uv inventory uses manager output for versions and receipts for ownership.
use super::{fingerprint, package, query, read, validate_name, ToolInstallation, UpdateRecipe};
use crate::domain::package::{InstalledPackage, PackageSource};
use anyhow::{Context, Result};
use std::{collections::BTreeMap, path::PathBuf};

pub(super) async fn inventory(
    manager: PathBuf,
    context: Option<&ToolInstallation>,
) -> Result<Vec<InstalledPackage>> {
    let (root, bin, output) = if let Some(context) = context {
        let root = context.root.clone();
        let bin = PathBuf::from(
            context
                .env
                .get("UV_TOOL_BIN_DIR")
                .context("uv bin directory missing")?,
        );
        let output = super::query_installation(
            context,
            &manager,
            &["tool", "list", "--color", "never", "--offline"],
        )
        .await?;
        (root, bin, output)
    } else {
        let root = PathBuf::from(query(&manager, &["tool", "dir"]).await?.trim());
        let bin = PathBuf::from(query(&manager, &["tool", "dir", "--bin"]).await?.trim());
        let output = query(&manager, &["tool", "list", "--color", "never", "--offline"]).await?;
        (root, bin, output)
    };
    let mut packages = Vec::new();
    for (id, version) in versions(&output)? {
        let environment = root.join(&id);
        let raw = read(&environment.join("uv-receipt.toml")).await?;
        let receipt: toml::Value = toml::from_str(&raw).context("Invalid uv tool receipt")?;
        let entries = receipt
            .get("tool")
            .and_then(|t| t.get("entrypoints"))
            .and_then(toml::Value::as_array)
            .context("uv receipt has no entry points")?;
        let executables = entries
            .iter()
            .map(|e| {
                let name = e
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .context("Missing uv entry point name")?;
                validate_name(name)?;
                let path = e
                    .get("install-path")
                    .and_then(toml::Value::as_str)
                    .context("Missing uv entry point path")?;
                Ok(PathBuf::from(path))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut env = BTreeMap::new();
        env.insert("UV_TOOL_DIR".into(), root.to_string_lossy().into_owned());
        env.insert("UV_TOOL_BIN_DIR".into(), bin.to_string_lossy().into_owned());
        let installation = ToolInstallation { source: PackageSource::Uv, manager: manager.clone(), root: root.clone(),
            environment: Some(environment), executables, env, protection_reason: None, receipt_hash: fingerprint(raw.as_bytes()), update: recipe(&receipt)?,
            notes: vec!["Upgrade respects this tool's original constraints. Shared caches, Python, and personal data are kept.".into()] };
        packages.push(package(&id, &version, installation)?);
    }
    Ok(packages)
}

pub(super) fn versions(output: &str) -> Result<Vec<(String, String)>> {
    let mut versions = Vec::new();
    for line in output
        .lines()
        .filter(|line| !line.starts_with('-') && !line.trim().is_empty())
    {
        let mut parts = line.split_whitespace();
        let id = parts.next().context("Missing uv tool name")?;
        validate_name(id)?;
        let version = parts
            .next()
            .and_then(|v| v.strip_prefix('v'))
            .context("Unexpected uv list output")?;
        versions.push((id.into(), version.into()));
    }
    Ok(versions)
}

fn recipe(receipt: &toml::Value) -> Result<UpdateRecipe> {
    let tool = receipt.get("tool").context("uv tool receipt missing")?;
    let render = |key: &str| -> Result<Vec<String>> {
        tool.get(key)
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .map(|requirement| {
                let name = requirement
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .context("uv requirement name missing")?;
                validate_name(name)?;
                let extras = requirement
                    .get("extras")
                    .and_then(toml::Value::as_array)
                    .map(|extras| {
                        extras
                            .iter()
                            .map(|v| {
                                let name = v.as_str().context("Invalid uv extra")?;
                                validate_name(name)?;
                                Ok(name.to_owned())
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                if ["git", "url", "path", "directory", "editable", "virtual"]
                    .iter()
                    .any(|key| requirement.get(key).is_some())
                {
                    anyhow::bail!("Git or local uv source has no registry release check");
                }
                if requirement.get("index").is_some() {
                    anyhow::bail!(
                        "Per-package uv index cannot be reproduced by this release check"
                    );
                }
                let specifier = requirement
                    .get("specifier")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("");
                if specifier.contains(['\n', '\r']) {
                    anyhow::bail!("Invalid uv version constraint");
                }
                let extras = if extras.is_empty() {
                    String::new()
                } else {
                    format!("[{}]", extras.join(","))
                };
                Ok(format!("{name}{extras}{specifier}"))
            })
            .collect()
    };
    let result = (|| {
        Ok::<_, anyhow::Error>(UpdateRecipe::Uv {
            requirements: render("requirements")?,
            constraints: render("constraints")?,
            overrides: render("overrides")?,
            resolver_config: tool.get("options").map(toml::to_string).transpose()?,
        })
    })();
    Ok(result.unwrap_or_else(|error| UpdateRecipe::Unsupported(error.to_string())))
}
