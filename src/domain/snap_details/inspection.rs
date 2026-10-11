//! Bounded inspection with progress updates for browsing and a fresh final result.

use super::{add_snapshots, api, data, from_revisions, unavailable, valid_name, SnapDetails};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub struct InspectionUpdate {
    pub details: SnapDetails,
    pub complete: bool,
}

/// Preview callers always wait for the complete fresh inspection.
pub async fn inspect(name: &str, cancel: Arc<AtomicBool>) -> SnapDetails {
    inspect_progressive(name, cancel, |_| {}).await
}

/// Publish paths before measurement, then each measured directory individually.
pub async fn inspect_progressive(
    name: &str,
    cancel: Arc<AtomicBool>,
    publish: impl Fn(InspectionUpdate) + Send + Sync + 'static,
) -> SnapDetails {
    if !valid_name(name) {
        return unavailable("Invalid Snap instance name");
    }
    let Some(permit) = crate::domain::package_details::inspection_permit().await else {
        return unavailable("Snap inspection is busy. Reopen details to try again.");
    };
    let permit = Arc::new(permit);
    let measurement_permit = permit.clone();
    if cancel.load(Ordering::Relaxed) {
        return unavailable("Inspection cancelled");
    }
    // Snapshot requests run alongside revisions and directory measurements.
    // A slow snapshot lookup must not hold back the main removal rows.
    let inspection = async {
        let details = from_revisions(name, api::revisions(name).await);
        if cancel.load(Ordering::Relaxed) {
            return details;
        }
        let name = name.to_owned();
        let fallback = details.clone();
        let result = tokio::task::spawn_blocking(move || {
            // Keep the permit until actual filesystem work ends.
            let _permit = measurement_permit;
            if cancel.load(Ordering::Relaxed) {
                return details;
            }
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
            let mut items = data::locations(home.as_deref(), &name);
            let mut current = details.clone();
            current.items.extend(items.clone());
            publish(InspectionUpdate {
                details: current,
                complete: false,
            });
            data::measure_items(&mut items, &cancel, |items| {
                if !cancel.load(Ordering::Relaxed) {
                    let mut current = details.clone();
                    current.items.extend_from_slice(items);
                    publish(InspectionUpdate {
                        details: current,
                        complete: false,
                    });
                }
            });
            let mut details = details;
            details.items.extend(items);
            details
        })
        .await;
        result.unwrap_or_else(|_| {
            let mut details = fallback;
            details
                .warnings
                .push("Managed data could not be measured".into());
            details
        })
    };
    let (mut details, snapshots) = tokio::join!(inspection, api::snapshots());
    add_snapshots(&mut details, name, snapshots);
    details
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires local snapd; reads metadata and managed directories only"]
    async fn live_inspection_publishes_paths_before_measured_sizes() {
        let updates = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = updates.clone();
        let final_details =
            inspect_progressive("snapd", Arc::new(AtomicBool::new(false)), move |update| {
                received.lock().unwrap().push(update);
            })
            .await;
        let updates = updates.lock().unwrap();
        assert!(!updates.is_empty());
        assert!(updates[0]
            .details
            .items
            .iter()
            .any(|item| item.size.note.as_deref() == Some(super::super::MEASURING_NOTE)));
        assert!(updates.iter().all(|update| !update.complete));
        assert!(final_details
            .items
            .iter()
            .all(|item| item.size.note.as_deref() != Some(super::super::MEASURING_NOTE)));
        assert!(final_details
            .items
            .iter()
            .any(|item| item.label.starts_with("Active revision")));
    }
}
