//! Measure attributable tool environments and binaries, excluding shared roots.
use super::{
    targets::{Discovery, Target},
    DetailEffect,
};
use crate::domain::{package::InstalledPackage, tool_managers};
use anyhow::{Context, Result};

pub(super) fn discover(pkg: &InstalledPackage) -> Result<Discovery> {
    let installation = pkg
        .tool_installation
        .as_ref()
        .context("Tool metadata missing; rescan first")?;
    tool_managers::validate(installation)?;
    let mut discovery = Discovery::default();
    if let Some(environment) = &installation.environment {
        discovery.add(
            if matches!(
                pkg.source,
                crate::domain::package::PackageSource::Npm
                    | crate::domain::package::PackageSource::Bun
            ) {
                "Package"
            } else {
                "Environment"
            },
            environment.display().to_string(),
            DetailEffect::Removed,
            Target::Directory(environment.clone()),
        );
        // Entry points are links/scripts into that measured environment; do not count their targets again.
        for path in &installation.executables {
            discovery.details.items.push(super::DetailItem {
                label: "Command".into(),
                location: Some(path.display().to_string()),
                effect: DetailEffect::Informational,
                size: super::Measurement::unknown("Exposed command; environment measured above"),
            });
        }
    } else {
        discovery.add(
            "Binaries",
            installation
                .executables
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
            DetailEffect::Removed,
            Target::OwnedFiles(installation.executables.clone()),
        );
    }
    if matches!(
        pkg.source,
        crate::domain::package::PackageSource::Npm | crate::domain::package::PackageSource::Bun
    ) {
        discovery.unknown("Shared dependencies", "Shared node_modules dependencies are not attributed to this package; removal effects depend on the manager");
    }
    discovery.unknown(
        "Shared data",
        "Shared caches, interpreters, and toolchains are kept",
    );
    discovery.unknown(
        "User data",
        "The tool receipt does not identify personal settings or app-created files",
    );
    Ok(discovery)
}
