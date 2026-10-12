//! Bounded update queries preserve installed rows when the network is unavailable.
use super::{query_installation, UpdateRecipe};
use crate::domain::{package::InstalledPackage, system};
use anyhow::{bail, Context, Result};

pub(super) async fn check(packages: &mut [InstalledPackage]) -> Option<String> {
    packages.first()?;
    for pkg in packages.iter_mut() {
        let protection = crate::domain::safety::check_package(pkg.source, &pkg.package_id);
        if protection.protected {
            pkg.update_note = protection.reason;
        }
    }
    // Two network workers per source, with a deadline per query.
    let mut tasks = tokio::task::JoinSet::new();
    let mut completed = vec![false; packages.len()];
    let mut next = 0;
    let mut failed = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        while tasks.len() < 2 && next < packages.len() {
            let pkg = packages[next].clone();
            let index = next;
            tasks.spawn(async move { (index, candidate(&pkg).await) });
            next += 1;
        }
        let result = match tokio::time::timeout_at(deadline, tasks.join_next()).await {
            Ok(Some(result)) => result,
            Ok(None) => break,
            Err(_) => {
                tasks.abort_all();
                for (_, pkg) in packages
                    .iter_mut()
                    .enumerate()
                    .filter(|(index, _)| !completed[*index])
                {
                    pkg.update_note
                        .get_or_insert_with(|| "Update check timed out. Rescan to retry.".into());
                }
                failed = true;
                break;
            }
        };
        match result {
            Ok((index, Ok(version))) => {
                completed[index] = true;
                packages[index].has_update = version.is_some();
                packages[index].update_version = version;
                if let Some(installation) = &packages[index].tool_installation {
                    if matches!(installation.update, UpdateRecipe::Pipx { pinned: true, .. }) {
                        packages[index].update_note =
                            Some("Pinned in pipx; unpin it there before updating.".into());
                    }
                }
            }
            Ok((index, Err(error))) => {
                completed[index] = true;
                failed = true;
                packages[index].update_note = Some(error.to_string());
            }
            Err(_) => failed = true,
        }
    }
    failed.then(|| {
        "Installed tools loaded; some update checks are unavailable. Expand a tool for details."
            .into()
    })
}

async fn candidate(pkg: &InstalledPackage) -> Result<Option<String>> {
    if crate::domain::safety::check_package(pkg.source, &pkg.package_id).protected {
        return Ok(None);
    }
    let installation = pkg
        .tool_installation
        .as_ref()
        .context("Tool installation missing")?;
    super::validate(installation)?;
    match &installation.update {
        UpdateRecipe::Javascript { .. } => {
            super::javascript_updates::candidate(pkg, installation).await
        }
        UpdateRecipe::Pipx { .. } | UpdateRecipe::Uv { .. } => {
            super::python_updates::candidate(pkg, installation).await
        }
        UpdateRecipe::Cargo {
            origin,
            version_req,
            ..
        } => {
            if origin == "registry+https://github.com/rust-lang/crates.io-index"
                || origin == "sparse+https://index.crates.io/"
            {
                let curl =
                    system::resolve("curl").context("curl is required for Cargo update checks")?;
                let address = format!("https://crates.io/api/v1/crates/{}", pkg.package_id);
                let output = query_installation(
                    installation,
                    &curl,
                    &[
                        "-fsSL",
                        "--max-time",
                        "20",
                        "-H",
                        "User-Agent: Scope package manager",
                        &address,
                    ],
                )
                .await?;
                registry_candidate(&output, &pkg.version, version_req.as_deref())
            } else if origin.starts_with("git+") {
                let (url, reference, pinned) = git_source(origin)?;
                if pinned {
                    return Ok(None);
                }
                let git = system::resolve("git").context("git is required for this source")?;
                let output = query_installation(
                    installation,
                    &git,
                    &["ls-remote", "--exit-code", &url, &reference],
                )
                .await?;
                let revision = output
                    .split_whitespace()
                    .next()
                    .context("Git revision unavailable")?;
                if !revision.bytes().all(|c| c.is_ascii_hexdigit()) || revision.len() != 40 {
                    bail!("Invalid Git revision");
                }
                Ok((origin.rsplit_once('#').map(|(_, v)| v) != Some(revision))
                    .then(|| format!("git:{revision}")))
            } else if origin.starts_with("path+") {
                bail!("Local Cargo source has no release check. Rebuild it with Cargo when its source changes.")
            } else {
                bail!("Custom Cargo registry update checks are unavailable; original source is retained.");
            }
        }
        UpdateRecipe::Unsupported(reason) => bail!("{reason}"),
    }
}

fn registry_candidate(
    output: &str,
    installed: &str,
    requirement: Option<&str>,
) -> Result<Option<String>> {
    let response: serde_json::Value = serde_json::from_str(output)?;
    let versions = response
        .get("versions")
        .and_then(|v| v.as_array())
        .context("Registry versions missing")?;
    let installed = semver::Version::parse(installed)?;
    let requirement = semver::VersionReq::parse(requirement.unwrap_or("*"))?;
    Ok(versions
        .iter()
        .filter(|v| v.get("yanked").and_then(|v| v.as_bool()) == Some(false))
        .filter_map(|v| {
            v.get("num")
                .and_then(|v| v.as_str())
                .and_then(|v| semver::Version::parse(v).ok())
        })
        .filter(|v| v > &installed && requirement.matches(v))
        .max()
        .map(|v| v.to_string()))
}

pub(super) fn git_source(origin: &str) -> Result<(String, String, bool)> {
    let mut url = url::Url::parse(origin.strip_prefix("git+").context("Not a Git source")?)?;
    let mut reference = "HEAD".to_owned();
    let mut pinned = false;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "branch" => reference = format!("refs/heads/{value}"),
            "tag" | "rev" => pinned = true,
            _ => bail!("Unsupported Git source selector"),
        }
    }
    url.set_fragment(None);
    url.set_query(None);
    if (url.scheme() != "ssh" && !url.username().is_empty()) || url.password().is_some() {
        bail!("Credential-bearing Git URLs are not supported in previews");
    }
    Ok((url.into(), reference, pinned))
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
