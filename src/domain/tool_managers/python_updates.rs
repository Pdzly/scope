//! Python tool candidates use the installed interpreter and recorded resolver settings.
use super::{query_installation, ToolInstallation, UpdateRecipe};
use crate::domain::package::InstalledPackage;
use anyhow::{bail, Context, Result};

pub(super) async fn candidate(
    pkg: &InstalledPackage,
    installation: &ToolInstallation,
) -> Result<Option<String>> {
    match &installation.update {
        UpdateRecipe::Pipx { pinned: true, .. } => Ok(None),
        UpdateRecipe::Pipx {
            package,
            pip_args,
            index_source,
            ..
        } => {
            if !index_source {
                bail!("This Git or local pipx source has no release-version check. Upgrade it through pipx.");
            }
            let python = installation
                .environment
                .as_ref()
                .context("pipx environment missing")?
                .join("bin/python");
            let mut args = vec![
                "-m".into(),
                "pip".into(),
                "list".into(),
                "--outdated".into(),
                "--format=json".into(),
                "--disable-pip-version-check".into(),
            ];
            // Exclude dependencies before querying indexes: a large GUI environment may contain hundreds.
            let local = query_installation(
                installation,
                &python,
                &[
                    "-m",
                    "pip",
                    "list",
                    "--format=json",
                    "--disable-pip-version-check",
                ],
            )
            .await?;
            args.extend(exclusions(&local, package)?);
            args.extend(index_arguments(pip_args)?);
            let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
            let output = query_installation(installation, &python, &refs).await?;
            pipx_candidate(&output, package)
        }
        UpdateRecipe::Uv {
            requirements,
            constraints,
            overrides,
            resolver_config,
        } => {
            let directory = tempfile::tempdir()?;
            let requirement_file = directory.path().join("requirements.txt");
            std::fs::write(&requirement_file, requirements.join("\n"))?;
            let python = installation
                .environment
                .as_ref()
                .context("uv environment missing")?
                .join("bin/python");
            let mut args = vec![
                "pip".into(),
                "compile".into(),
                "--upgrade".into(),
                "--no-build".into(),
                "--no-header".into(),
                "--no-annotate".into(),
                "--python".into(),
                python.display().to_string(),
                requirement_file.display().to_string(),
                "--color".into(),
                "never".into(),
                "--no-progress".into(),
            ];
            for (filename, flag, rows) in [
                ("constraints.txt", "--constraint", constraints),
                ("overrides.txt", "--override", overrides),
            ] {
                if !rows.is_empty() {
                    let path = directory.path().join(filename);
                    std::fs::write(&path, rows.join("\n"))?;
                    args.extend([flag.into(), path.display().to_string()]);
                }
            }
            if let Some(config) = resolver_config {
                let path = directory.path().join("uv.toml");
                std::fs::write(&path, config)?;
                args.extend(["--config-file".into(), path.display().to_string()]);
            }
            let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
            let output = query_installation(installation, &installation.manager, &refs).await
                .context("Update resolution unavailable. Original constraints and source are retained; rescan to retry.")?;
            let version = output
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{}==", pkg.package_id)))
                .map(str::trim)
                .context("Resolved uv version missing")?;
            let installed = pkg.version.parse::<pep440_rs::Version>()?;
            let candidate = version.parse::<pep440_rs::Version>()?;
            Ok((candidate > installed).then(|| version.to_owned()))
        }
        _ => bail!("Not a Python tool recipe"),
    }
}

fn index_arguments(args: &[String]) -> Result<Vec<String>> {
    let mut result = Vec::new();
    let mut parts = args.iter();
    while let Some(arg) = parts.next() {
        if [
            "-i",
            "-f",
            "--no-binary",
            "--only-binary",
            "--index-url",
            "--extra-index-url",
            "--find-links",
            "--trusted-host",
            "--cert",
            "--client-cert",
        ]
        .contains(&arg.as_str())
        {
            result.push(arg.clone());
            result.push(parts.next().context("Missing pip index argument")?.clone());
        } else if (arg.len() > 2 && (arg.starts_with("-i") || arg.starts_with("-f")))
            || arg == "--no-index"
            || arg == "--pre"
            || arg == "--prefer-binary"
            || [
                "--no-binary=",
                "--only-binary=",
                "--index-url=",
                "--extra-index-url=",
                "--find-links=",
                "--trusted-host=",
                "--cert=",
                "--client-cert=",
            ]
            .iter()
            .any(|prefix| arg.starts_with(prefix))
        {
            result.push(arg.clone());
        }
    }
    Ok(result)
}

pub(super) fn pipx_candidate(output: &str, package: &str) -> Result<Option<String>> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(output).context("Invalid pip update inventory")?;
    Ok(rows
        .iter()
        .find(|row| {
            row.get("name")
                .and_then(|s| s.as_str())
                .is_some_and(|name| normalize(name) == normalize(package))
        })
        .and_then(|row| row.get("latest_version"))
        .and_then(|v| v.as_str())
        .map(str::to_owned))
}

fn normalize(name: &str) -> String {
    name.to_ascii_lowercase().replace(['_', '.'], "-")
}

fn exclusions(output: &str, package: &str) -> Result<Vec<String>> {
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(output).context("Invalid local pip inventory")?;
    let mut args = Vec::new();
    let mut present = false;
    for row in rows {
        let name = row
            .get("name")
            .and_then(|value| value.as_str())
            .context("pip distribution name missing")?;
        if normalize(name) == normalize(package) {
            present = true;
        } else {
            args.extend(["--exclude".into(), name.into()]);
        }
    }
    if !present {
        bail!("The pipx main distribution is missing from its environment");
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excludes_dependencies_and_requires_the_main_distribution() {
        let rows = r#"[{"name":"demo_tool"},{"name":"library"},{"name":"pip"}]"#;
        assert_eq!(
            exclusions(rows, "demo-tool").unwrap(),
            vec!["--exclude", "library", "--exclude", "pip"]
        );
        assert!(exclusions(rows, "missing").is_err());
    }
    #[test]
    fn keeps_short_index_options_and_filters_install_only_arguments() {
        let args = vec![
            "-i".into(),
            "https://example.com/simple".into(),
            "--pre".into(),
            "--upgrade".into(),
        ];
        assert_eq!(
            index_arguments(&args).unwrap(),
            vec!["-i", "https://example.com/simple", "--pre"]
        );
    }
}
