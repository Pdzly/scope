//! Known Snap data locations. Alternative layouts need explicit coverage labels.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{measurement, DetailItem, Measurement, RemovalEffect};

pub(super) fn locations(home: Option<&Path>, name: &str) -> Vec<DetailItem> {
    let mut items = Vec::new();
    if let Some(home) = home {
        let standard = home.join("snap").join(name);
        let hidden = home.join(".snap/data").join(name);
        let hidden_exists = match hidden.symlink_metadata() {
            Ok(_) => true,
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        };
        // Hidden data is an experimental snapd layout. The active per-snap
        // migration policy is not exposed by our unauthenticated metadata read.
        if hidden_exists {
            if standard.symlink_metadata().is_ok() {
                items.push(item(
                    "Your default data directory",
                    &standard,
                    RemovalEffect::Unverified,
                ));
            }
            let hidden_item = item(
                "Your hidden data directory",
                &hidden,
                RemovalEffect::Unverified,
            );
            items.push(hidden_item);
        } else {
            items.push(item(
                "Your managed data",
                &standard,
                RemovalEffect::RemovedBySnap,
            ));
        }
    } else {
        items.push(DetailItem {
            label: "Your managed data".into(),
            location: None,
            size: Measurement::unknown("Home directory unavailable"),
            effect: RemovalEffect::Unverified,
        });
    }
    items.push(item(
        "System managed data",
        &Path::new("/var/snap").join(name),
        RemovalEffect::RemovedBySnap,
    ));
    items
}

fn item(label: &str, path: &Path, effect: RemovalEffect) -> DetailItem {
    DetailItem {
        label: label.into(),
        location: Some(path.to_string_lossy().into_owned()),
        size: Measurement::unknown(super::MEASURING_NOTE),
        effect,
    }
}

/// Publish each completed size without waiting for the remaining directories.
pub(super) fn measure_items(
    items: &mut [DetailItem],
    cancel: &AtomicBool,
    mut publish: impl FnMut(&[DetailItem]),
) {
    for index in 0..items.len() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Some(location) = items[index].location.as_deref() else {
            continue;
        };
        let mut size = measurement::measure(Path::new(location), cancel);
        if items[index].effect == RemovalEffect::Unverified {
            let note = size.note.take().unwrap_or_default();
            size.note = Some(format!(
                "Alternative Snap layout; removal coverage could not be verified. {note}"
            ));
        }
        items[index].size = size;
        publish(items);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_each_size_while_later_rows_remain_pending() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("snap/code")).unwrap();
        let mut rows = locations(Some(home.path()), "code");
        assert!(rows
            .iter()
            .all(|item| item.size.note.as_deref() == Some(super::super::MEASURING_NOTE)));
        let mut updates = Vec::new();
        measure_items(&mut rows, &AtomicBool::new(false), |items| {
            updates.push(items.to_vec())
        });
        assert_eq!(updates.len(), 2);
        assert!(updates[0][0].size.bytes.is_some());
        assert_eq!(
            updates[0][1].size.note.as_deref(),
            Some(super::super::MEASURING_NOTE)
        );
        assert!(updates[1][1].size.bytes.is_some());
    }

    #[test]
    fn cancelled_measurement_publishes_no_results() {
        let mut rows = locations(None, "code");
        measure_items(&mut rows, &AtomicBool::new(true), |_| {
            panic!("Canceled work must not publish")
        });
        assert_eq!(rows[1].size.bytes, None);
    }

    #[test]
    fn alternative_layout_is_measured_without_promising_removal() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".snap/data/code")).unwrap();
        let mut items = locations(Some(home.path()), "code");
        measure_items(&mut items, &AtomicBool::new(false), |_| {});
        assert_eq!(items[0].effect, RemovalEffect::Unverified);
        assert!(items[0].size.bytes.is_some());
        assert_eq!(items[0].label, "Your hidden data directory");
    }
}
