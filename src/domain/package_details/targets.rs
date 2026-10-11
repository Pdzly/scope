//! One measurement engine shared by the non-Snap source inspectors.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{
    inspection::{InspectionUpdate, Publish},
    measurement,
    owned_files::OwnedMeasurements,
    DetailEffect, DetailItem, Measurement, PackageDetails, MEASURING_NOTE,
};

pub(super) enum Target {
    Directory(PathBuf),
    File(PathBuf),
    OwnedFiles(Vec<PathBuf>),
}

#[derive(Default)]
pub(super) struct Discovery {
    pub details: PackageDetails,
    targets: Vec<(usize, Target)>,
}

impl Discovery {
    pub fn add(&mut self, label: &str, location: String, effect: DetailEffect, target: Target) {
        self.targets.push((self.details.items.len(), target));
        self.details.items.push(DetailItem {
            label: label.into(),
            location: Some(location),
            effect,
            size: Measurement::unknown(MEASURING_NOTE),
        });
    }

    pub fn unknown(&mut self, label: &str, note: &str) {
        self.details.items.push(DetailItem {
            label: label.into(),
            location: None,
            effect: DetailEffect::Informational,
            size: Measurement::unknown(note),
        });
    }
}

pub(super) fn measure(
    mut discovery: Discovery,
    cancel: &AtomicBool,
    publish: &Publish,
) -> PackageDetails {
    let mut owned = OwnedMeasurements::default();
    if !cancel.load(Ordering::Relaxed) {
        publish(InspectionUpdate {
            details: discovery.details.clone(),
            complete: false,
        });
    }
    for (index, target) in discovery.targets {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        discovery.details.items[index].size = match target {
            Target::Directory(path) => measurement::measure(&path, cancel),
            Target::File(path) => measurement::file_size(&path),
            Target::OwnedFiles(paths) => owned.measure(&paths, cancel),
        };
        if !cancel.load(Ordering::Relaxed) {
            publish(InspectionUpdate {
                details: discovery.details.clone(),
                complete: false,
            });
        }
    }
    discovery.details
}
