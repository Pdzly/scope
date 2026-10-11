//! Measure only explicitly owned files, never their shared parent directories.

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::Measurement;

pub(super) struct OwnedMeasurements {
    seen: HashSet<(u64, u64)>,
    deadline: Instant,
    remaining: usize,
}

impl Default for OwnedMeasurements {
    fn default() -> Self {
        Self {
            seen: HashSet::new(),
            deadline: Instant::now() + Duration::from_secs(2),
            remaining: 20_000,
        }
    }
}

impl OwnedMeasurements {
    pub fn measure(&mut self, files: &[PathBuf], cancel: &AtomicBool) -> Measurement {
        let mut bytes = 0_u64;
        let mut complete = true;
        let mut known = false;
        for path in files {
            if cancel.load(Ordering::Relaxed)
                || Instant::now() >= self.deadline
                || self.remaining == 0
            {
                complete = false;
                break;
            }
            self.remaining -= 1;
            let metadata = match path.symlink_metadata() {
                Ok(metadata) => metadata,
                Err(_) => {
                    complete = false;
                    continue;
                }
            };
            if metadata.is_dir() {
                continue;
            }
            known = true;
            if self.seen.insert((metadata.dev(), metadata.ino())) {
                let Some(total) = metadata
                    .blocks()
                    .checked_mul(512)
                    .and_then(|size| bytes.checked_add(size))
                else {
                    return Measurement::unknown("File sizes exceeded the measurement limit");
                };
                bytes = total;
            }
        }
        Measurement {
            bytes: known.then_some(bytes),
            complete: known && complete,
            note: (!complete).then(|| "Some package files could not be measured".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_unowned_neighbors_and_counts_hardlinks_once_across_groups() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("owned");
        let link = root.path().join("hardlink");
        std::fs::write(&file, vec![1; 8192]).unwrap();
        std::fs::hard_link(&file, &link).unwrap();
        std::fs::write(root.path().join("unowned"), vec![1; 65536]).unwrap();
        let mut sizes = OwnedMeasurements::default();
        let cancel = AtomicBool::new(false);
        let first = sizes.measure(std::slice::from_ref(&file), &cancel);
        assert_eq!(first.bytes, Some(file.metadata().unwrap().blocks() * 512));
        assert_eq!(sizes.measure(&[link], &cancel).bytes, Some(0));
    }

    #[test]
    fn unavailable_files_and_exhausted_budget_remain_unknown() {
        let mut sizes = OwnedMeasurements {
            remaining: 0,
            ..OwnedMeasurements::default()
        };
        assert_eq!(
            sizes
                .measure(&[PathBuf::from("/missing")], &AtomicBool::new(false))
                .bytes,
            None
        );
    }
}
