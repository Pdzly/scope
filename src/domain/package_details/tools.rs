//! Measure attributable tool environments and binaries, excluding shared roots.
use super::{
    targets::{Discovery, Target},
    DetailEffect, Measurement, MeasurementStatus,
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
            Target::ToolDirectory(environment.clone()),
        );
        // Entry points are links/scripts into that measured environment; do not count their targets again.
        for path in &installation.executables {
            discovery.details.items.push(super::DetailItem {
                label: "Command".into(),
                location: Some(path.display().to_string()),
                effect: DetailEffect::Informational,
                size: Measurement::descriptive(MeasurementStatus::Included, "Command target included in the package or environment above; not counted twice"),
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
            Target::ToolFiles(installation.executables.clone()),
        );
    }
    if matches!(
        pkg.source,
        crate::domain::package::PackageSource::Npm | crate::domain::package::PackageSource::Bun
    ) {
        descriptive(&mut discovery, "Shared dependencies", DetailEffect::Informational, MeasurementStatus::Unattributed,
            "Shared node_modules dependencies cannot be assigned to this package; the manager decides which are removed");
    }
    descriptive(
        &mut discovery,
        "Shared data",
        DetailEffect::Kept,
        MeasurementStatus::NotMeasured,
        "Shared caches, interpreters, and toolchains are kept",
    );
    descriptive(
        &mut discovery,
        "User data",
        DetailEffect::Informational,
        MeasurementStatus::Unattributed,
        "The tool receipt does not identify personal settings or app-created files",
    );
    Ok(discovery)
}

fn descriptive(
    discovery: &mut Discovery,
    label: &str,
    effect: DetailEffect,
    status: MeasurementStatus,
    note: &str,
) {
    discovery.details.items.push(super::DetailItem {
        label: label.into(),
        location: None,
        effect,
        size: Measurement::descriptive(status, note),
    });
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
