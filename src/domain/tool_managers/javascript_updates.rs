//! Native registry resolution keeps npm version ranges and Bun tags out of Rust's Cargo parser.
use super::{home, query_installation, ToolInstallation, UpdateRecipe};
use crate::domain::{
    package::{InstalledPackage, PackageSource},
    system,
};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::time::Duration;

pub(super) async fn candidate(
    pkg: &InstalledPackage,
    installation: &ToolInstallation,
) -> Result<Option<String>> {
    let UpdateRecipe::Javascript {
        requirement,
        registry_source,
        ..
    } = &installation.update
    else {
        bail!("Global update metadata missing");
    };
    if !registry_source {
        bail!("Git, local, linked, and aliased JavaScript installs have no supported registry update check");
    }
    if pkg.source == PackageSource::Npm {
        let prefix = installation
            .root
            .to_str()
            .context("npm prefix is not UTF-8")?;
        let env = installation
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>();
        let output = system::capture_output_context(
            installation
                .manager
                .to_str()
                .context("Manager path is not UTF-8")?,
            &[
                "outdated",
                "--global",
                "--prefix",
                prefix,
                "--json",
                pkg.package_id.as_str(),
            ],
            &env,
            Some(&home()?),
            Duration::from_secs(30),
        )
        .await?;
        // npm deliberately exits 1 when it finds outdated packages; only valid metadata is usable.
        if !matches!(output.exit_code, Some(0 | 1)) {
            bail!("npm update check failed");
        }
        npm_candidate(&output.stdout, &pkg.package_id, &pkg.version)
    } else {
        let spec = format!("{}@{requirement}", pkg.package_id);
        let output = query_installation(
            installation,
            &installation.manager,
            &["info", &spec, "version", "--json"],
        )
        .await?;
        bun_candidate(&output, &pkg.version)
    }
}

pub(super) fn npm_candidate(output: &str, id: &str, installed: &str) -> Result<Option<String>> {
    let value: Value = serde_json::from_str(output).context("Invalid npm update response")?;
    let entries = value
        .as_object()
        .context("npm update response is not an object")?;
    if entries.contains_key("error") {
        bail!("npm registry update check is unavailable");
    }
    let Some(entry) = entries.get(id) else {
        if entries.is_empty() {
            return Ok(None);
        }
        bail!("npm returned an unrelated package update");
    };
    if entry.get("current").and_then(Value::as_str) != Some(installed) {
        bail!("Installed npm version changed during update check");
    }
    let wanted = entry
        .get("wanted")
        .and_then(Value::as_str)
        .context("npm update target missing")?;
    newer(wanted, installed)
}

pub(super) fn bun_candidate(output: &str, installed: &str) -> Result<Option<String>> {
    let value: Value = serde_json::from_str(output).context("Invalid Bun registry response")?;
    let versions = match &value {
        Value::String(version) => vec![version.as_str()],
        Value::Array(versions) => versions
            .iter()
            .map(|version| version.as_str().context("Invalid Bun version"))
            .collect::<Result<Vec<_>>>()?,
        _ => bail!("Bun registry version is unavailable"),
    };
    let candidate = versions
        .iter()
        .map(|version| semver::Version::parse(version))
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .context("Bun registry returned no matching versions")?;
    newer(&candidate.to_string(), installed)
}

fn newer(candidate: &str, installed: &str) -> Result<Option<String>> {
    let candidate =
        semver::Version::parse(candidate).context("Registry target is not a release version")?;
    Ok((candidate > semver::Version::parse(installed)?).then(|| candidate.to_string()))
}
