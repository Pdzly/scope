//! Global-only command argv shared by preview and apply.
use super::{ToolInstallation, UpdateRecipe};
use crate::domain::{operations::Operation, package::PackageSource};
use anyhow::{bail, Context, Result};

pub(super) fn command(
    id: &str,
    installation: &ToolInstallation,
    operation: Operation,
) -> Result<Vec<String>> {
    let UpdateRecipe::Javascript {
        registry_source, ..
    } = &installation.update
    else {
        bail!("Global tool metadata missing");
    };
    let environment = installation
        .environment
        .as_ref()
        .context("Global package directory missing")?;
    if environment != &super::javascript::modules(installation.source, &installation.root).join(id)
    {
        bail!("Operation package disagrees with its recorded directory");
    }
    if operation == Operation::Update && !registry_source {
        bail!("Registry updates are unsupported for this JavaScript source");
    }
    Ok(if installation.source == PackageSource::Npm {
        vec![
            if operation == Operation::Update {
                "update"
            } else {
                "uninstall"
            }
            .into(),
            "--global".into(),
            "--prefix".into(),
            installation.root.display().to_string(),
            id.into(),
        ]
    } else {
        vec![
            if operation == Operation::Update {
                "update"
            } else {
                "remove"
            }
            .into(),
            "--global".into(),
            id.into(),
        ]
    })
}
