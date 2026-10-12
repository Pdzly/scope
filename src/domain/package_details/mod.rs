//! Source-aware, read-only installed footprints. Rows never authorize deletion.

mod apt;
mod flatpak;
mod inspection;
mod local;
pub(crate) mod measurement;
mod owned_files;
mod snap;
mod targets;
mod tools;

use serde::{Deserialize, Serialize};

pub(crate) use inspection::inspection_permit;
pub use inspection::{inspect_progressive, InspectionUpdate};
pub use measurement::{Measurement, MeasurementStatus};

pub const MEASURING_NOTE: &str = "Measuring…";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetailEffect {
    Removed,
    Kept,
    Unverified,
    /// A detected file or footprint without an uninstall ownership claim.
    Informational,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailItem {
    pub label: String,
    pub location: Option<String>,
    pub size: Measurement,
    pub effect: DetailEffect,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageDetails {
    pub items: Vec<DetailItem>,
    pub warnings: Vec<String>,
}

impl PackageDetails {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            items: vec![],
            warnings: vec![message.into()],
        }
    }

    /// Only sizes of affected package-owned items, never net freed disk space.
    pub fn removal_size(&self) -> Measurement {
        let mut bytes = None::<u64>;
        let mut complete = self.warnings.is_empty();
        for item in self
            .items
            .iter()
            .filter(|item| item.effect == DetailEffect::Removed)
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
            status: Default::default(),
            bytes,
            complete: complete && bytes.is_some(),
            note: None,
        }
    }
}
