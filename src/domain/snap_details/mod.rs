//! Snap removal consequences. These are informational and never deletion targets.

mod api;
mod data;
mod inspection;
mod removal_size;

use serde::{Deserialize, Serialize};

use crate::domain::package_details::measurement;
pub use crate::domain::package_details::Measurement;
pub use inspection::{inspect, inspect_progressive};

pub const CLASSIC_COVERAGE_NOTE: &str = "This classic snap can store files outside Snap's managed directories. Those files are outside this breakdown.";
pub const NO_SNAPSHOTS_NOTE: &str = "No existing recovery snapshots were reported for this app.";
pub use crate::domain::package_details::MEASURING_NOTE;

/// A manager-owned item and what normal Snap removal does to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailItem {
    pub label: String,
    pub location: Option<String>,
    pub size: Measurement,
    pub effect: RemovalEffect,
}

/// Ownership consequences of normal removal. Unverified paths are informational.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemovalEffect {
    RemovedBySnap,
    Kept,
    Unverified,
}

/// Fresh local metadata plus bounded measurements of the current user's data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapDetails {
    pub summary: Option<String>,
    pub items: Vec<DetailItem>,
    pub warnings: Vec<String>,
}

fn unavailable(message: &str) -> SnapDetails {
    SnapDetails {
        summary: None,
        items: Vec::new(),
        warnings: vec![message.into()],
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn manager_size(bytes: Option<u64>) -> Measurement {
    Measurement {
        bytes,
        complete: bytes.is_some(),
        note: Some("Snap-reported size".into()),
    }
}

#[cfg(test)]
fn from_metadata(
    name: &str,
    revisions: anyhow::Result<Vec<api::Revision>>,
    snapshots: anyhow::Result<Vec<api::SnapshotSet>>,
) -> SnapDetails {
    let mut details = from_revisions(name, revisions);
    add_snapshots(&mut details, name, snapshots);
    details
}

fn from_revisions(name: &str, revisions: anyhow::Result<Vec<api::Revision>>) -> SnapDetails {
    let mut details = SnapDetails {
        summary: None,
        items: Vec::new(),
        warnings: Vec::new(),
    };
    match revisions {
        Ok(revisions) => {
            let mut revisions: Vec<_> = revisions
                .into_iter()
                .filter(|revision| revision.name == name)
                .collect();
            revisions.sort_by_key(|revision| revision.status != "active");
            if revisions.is_empty() {
                details
                    .warnings
                    .push("No installed revisions were reported. Rescan to check this app.".into());
            }
            for revision in revisions {
                if revision.status == "active" {
                    details.summary = revision.summary;
                }
                if revision.trymode {
                    details.warnings.push("This snap runs from a development directory. Its source files are outside normal removal coverage.".into());
                }
                if revision.confinement.as_deref() == Some("classic") {
                    let warning = CLASSIC_COVERAGE_NOTE.to_owned();
                    if !details.warnings.contains(&warning) {
                        details.warnings.push(warning);
                    }
                }
                let status = if revision.status == "active" {
                    "Active"
                } else {
                    "Older"
                };
                details.items.push(DetailItem {
                    label: format!("{status} revision {}", revision.revision),
                    location: revision.path,
                    size: manager_size(revision.size),
                    effect: if revision.trymode {
                        RemovalEffect::Kept
                    } else {
                        RemovalEffect::RemovedBySnap
                    },
                });
                for component in revision
                    .components
                    .into_iter()
                    .filter(|component| component.revision.is_some())
                {
                    details.items.push(DetailItem {
                        label: format!(
                            "Component {} · revision {}",
                            component.name,
                            component.revision.unwrap_or_default()
                        ),
                        location: None,
                        size: manager_size(component.size),
                        effect: RemovalEffect::RemovedBySnap,
                    });
                }
            }
        }
        Err(_) => details
            .warnings
            .push("Application revisions could not be read from Snap.".into()),
    }
    details
}

fn add_snapshots(
    details: &mut SnapDetails,
    name: &str,
    snapshots: anyhow::Result<Vec<api::SnapshotSet>>,
) {
    match snapshots {
        Ok(sets) => {
            let before = details.items.len();
            for set in sets {
                for snapshot in set
                    .snapshots
                    .into_iter()
                    .filter(|snapshot| snapshot.snap == name)
                {
                    details.items.push(DetailItem {
                        label: format!("Recovery snapshot · set {}", set.id),
                        location: Some("/var/lib/snapd/snapshots".into()),
                        size: manager_size(snapshot.size),
                        effect: RemovalEffect::Kept,
                    });
                }
            }
            if before == details.items.len() {
                details.warnings.push(NO_SNAPSHOTS_NOTE.into());
            }
        }
        Err(_) => details
            .warnings
            .push("Existing recovery snapshots could not be read from Snap.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicBool, Arc};

    #[tokio::test]
    async fn cancelled_inspection_does_not_read_metadata_or_directories() {
        let details = inspect("code", Arc::new(AtomicBool::new(true))).await;
        assert!(details.items.is_empty());
        assert_eq!(details.warnings, ["Inspection cancelled"]);
    }

    #[tokio::test]
    #[ignore = "requires a running local snapd; reads metadata and managed directories only"]
    async fn inspects_live_snapd_without_modifying_it() {
        let details = inspect("snapd", Arc::new(AtomicBool::new(false))).await;
        assert!(details
            .items
            .iter()
            .any(|item| item.label.starts_with("Active revision") && item.size.bytes.is_some()));
        assert!(details
            .items
            .iter()
            .any(|item| item.label == "Your managed data"));
        assert!(!details
            .warnings
            .iter()
            .any(|warning| warning.contains("could not be read")));
    }

    #[test]
    fn rejects_names_that_can_escape_managed_paths_or_change_the_query() {
        for name in ["../code", "/tmp/code", "code&select=enabled", "", "-code"] {
            assert!(!valid_name(name));
        }
        assert!(valid_name("code_test"));
    }

    #[test]
    fn separates_revisions_components_and_retained_snapshots() {
        let revisions = serde_json::from_str(r#"[{"name":"code","revision":"1","status":"installed","installed-size":10},{"name":"code","revision":"2","status":"active","installed-size":20,"summary":"Editor","components":[{"name":"extra","revision":"3","installed-size":5},{"name":"uninstalled"}]},{"name":"other","revision":"1","status":"active"}]"#).unwrap();
        let snapshots = serde_json::from_str(
            r#"[{"id":7,"snapshots":[{"snap":"code","size":4},{"snap":"other","size":999}]}]"#,
        )
        .unwrap();
        let details = from_metadata("code", Ok(revisions), Ok(snapshots));
        assert_eq!(details.items.len(), 4);
        assert_eq!(details.summary.as_deref(), Some("Editor"));
        assert_eq!(details.items[3].effect, RemovalEffect::Kept);
        assert_eq!(details.items[0].effect, RemovalEffect::RemovedBySnap);
    }

    #[test]
    fn failed_metadata_is_not_treated_as_no_snapshots_or_zero_size() {
        let details = from_metadata(
            "code",
            Err(anyhow::anyhow!("offline")),
            Err(anyhow::anyhow!("denied")),
        );
        assert!(details.items.is_empty());
        assert_eq!(details.warnings.len(), 2);
        assert!(manager_size(None).bytes.is_none());
    }

    #[test]
    fn development_source_is_kept_and_classic_coverage_is_explained() {
        let revisions = serde_json::from_str(r#"[{"name":"code","revision":"x1","status":"active","trymode":true,"confinement":"classic","mounted-from":"/tmp/dev-code"}]"#).unwrap();
        let details = from_metadata("code", Ok(revisions), Ok(vec![]));
        assert_eq!(details.items[0].effect, RemovalEffect::Kept);
        assert!(details
            .warnings
            .iter()
            .any(|warning| warning.contains("classic")));
    }
}
