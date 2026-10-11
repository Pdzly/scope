//! Estimated managed items affected by removal, never guaranteed freed space.

use super::{Measurement, RemovalEffect, SnapDetails};

impl SnapDetails {
    /// Sum removed items only. Snapshots can retain data after normal removal.
    pub fn removal_size(&self) -> Measurement {
        let mut bytes = None::<u64>;
        let mut complete = self.warnings.iter().all(|warning| {
            warning == super::CLASSIC_COVERAGE_NOTE || warning == super::NO_SNAPSHOTS_NOTE
        });
        for item in self
            .items
            .iter()
            .filter(|item| item.effect == RemovalEffect::RemovedBySnap)
        {
            complete &= item.size.complete;
            if let Some(size) = item.size.bytes {
                let Some(total) = bytes.unwrap_or(0).checked_add(size) else {
                    return Measurement::unknown("Removal size exceeds the supported range");
                };
                bytes = Some(total);
            } else {
                complete = false;
            }
        }
        Measurement {
            bytes,
            complete: complete && bytes.is_some(),
            note: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::snap_details::DetailItem;

    fn item(bytes: Option<u64>, complete: bool, effect: RemovalEffect) -> DetailItem {
        DetailItem {
            label: "Test item".into(),
            location: None,
            size: Measurement {
                bytes,
                complete,
                note: None,
            },
            effect,
        }
    }

    #[test]
    fn estimate_excludes_retained_and_unverified_items() {
        let details = SnapDetails {
            summary: None,
            warnings: vec![
                super::super::CLASSIC_COVERAGE_NOTE.into(),
                super::super::NO_SNAPSHOTS_NOTE.into(),
            ],
            items: vec![
                item(Some(100), true, RemovalEffect::RemovedBySnap),
                item(Some(20), true, RemovalEffect::RemovedBySnap),
                item(Some(400), true, RemovalEffect::Kept),
                item(Some(500), true, RemovalEffect::Unverified),
            ],
        };
        let size = details.removal_size();
        assert_eq!(size.bytes, Some(120));
        assert!(size.complete);
    }

    #[test]
    fn partial_measurements_stay_incomplete() {
        let details = SnapDetails {
            summary: None,
            warnings: vec![],
            items: vec![
                item(Some(100), false, RemovalEffect::RemovedBySnap),
                item(None, false, RemovalEffect::RemovedBySnap),
            ],
        };
        let size = details.removal_size();
        assert_eq!(size.bytes, Some(100));
        assert!(!size.complete);
    }

    #[test]
    fn missing_inspection_and_overflow_never_become_zero_or_a_wrapped_total() {
        let mut details = SnapDetails {
            summary: None,
            warnings: vec!["Inspection failed".into()],
            items: vec![],
        };
        assert_eq!(details.removal_size().bytes, None);
        details
            .items
            .push(item(Some(100), true, RemovalEffect::RemovedBySnap));
        assert!(!details.removal_size().complete);
        details
            .items
            .push(item(Some(u64::MAX), true, RemovalEffect::RemovedBySnap));
        assert_eq!(details.removal_size().bytes, None);
    }
}
