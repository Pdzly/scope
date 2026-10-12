#[cfg(test)]
mod tests {
    use super::super::{Measurement, RemovalEffect, SnapDetails};
    use crate::domain::package_details::PackageDetails;
    use crate::domain::snap_details::DetailItem;

    fn item(bytes: Option<u64>, complete: bool, effect: RemovalEffect) -> DetailItem {
        DetailItem {
            label: "Test item".into(),
            location: None,
            size: Measurement {
                status: Default::default(),
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
        let size = PackageDetails::from(&details).removal_size();
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
        let size = PackageDetails::from(&details).removal_size();
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
        assert_eq!(PackageDetails::from(&details).removal_size().bytes, None);
        details
            .items
            .push(item(Some(100), true, RemovalEffect::RemovedBySnap));
        assert!(!PackageDetails::from(&details).removal_size().complete);
        details
            .items
            .push(item(Some(u64::MAX), true, RemovalEffect::RemovedBySnap));
        assert_eq!(PackageDetails::from(&details).removal_size().bytes, None);
    }
}
