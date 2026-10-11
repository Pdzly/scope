//! Adapt existing Snap inspection without changing operation-plan metadata.

use super::{DetailEffect, DetailItem, PackageDetails};
use crate::domain::snap_details::{
    RemovalEffect, SnapDetails, CLASSIC_COVERAGE_NOTE, NO_SNAPSHOTS_NOTE,
};

impl From<&SnapDetails> for PackageDetails {
    fn from(details: &SnapDetails) -> Self {
        Self {
            items: details
                .items
                .iter()
                .map(|item| {
                    let label = if item.label.starts_with("Active revision") {
                        "Application"
                    } else if item.label.starts_with("Older revision") {
                        "Older revision"
                    } else if item.label.starts_with("Component") {
                        "Component"
                    } else if item.label.starts_with("Recovery snapshot") {
                        "Recovery snapshot"
                    } else if item.label.starts_with("System") {
                        "System data"
                    } else {
                        "User data"
                    };
                    let location = if label == "Recovery snapshot" {
                        Some(format!(
                            "{} · {}",
                            item.location.as_deref().unwrap_or("Snapshot"),
                            item.label
                        ))
                    } else {
                        item.location.clone().or_else(|| Some(item.label.clone()))
                    };
                    DetailItem {
                        label: label.into(),
                        location,
                        size: item.size.clone(),
                        effect: match item.effect {
                            RemovalEffect::RemovedBySnap => DetailEffect::Removed,
                            RemovalEffect::Kept => DetailEffect::Kept,
                            RemovalEffect::Unverified => DetailEffect::Unverified,
                        },
                    }
                })
                .collect(),
            warnings: details
                .warnings
                .iter()
                .filter(|note| {
                    note.as_str() != CLASSIC_COVERAGE_NOTE && note.as_str() != NO_SNAPSHOTS_NOTE
                })
                .cloned()
                .collect(),
        }
    }
}
