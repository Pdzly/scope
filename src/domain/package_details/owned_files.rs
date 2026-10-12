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
    pub(super) fn tool() -> Self {
        Self {
            deadline: Instant::now() + super::measurement::TOOL_TIME_LIMIT,
            remaining: super::measurement::TOOL_ENTRY_LIMIT,
            ..Self::default()
        }
    }

    pub fn measure(&mut self, files: &[PathBuf], cancel: &AtomicBool) -> Measurement {
        self.measure_progressive(files, cancel, |_| {})
    }

    pub(super) fn measure_progressive(
        &mut self,
        files: &[PathBuf],
        cancel: &AtomicBool,
        mut progress: impl FnMut(Measurement),
    ) -> Measurement {
        let mut bytes = 0_u64;
        let mut complete = true;
        let mut known = false;
        let mut note = None;
        let mut hard_links = false;
        let mut last_progress = Instant::now();
        for (index, path) in files.iter().enumerate() {
            let stopped = if cancel.load(Ordering::Relaxed) {
                Some("Inspection cancelled before all owned files were measured")
            } else if Instant::now() >= self.deadline {
                Some("Owned-file measurement reached its time limit")
            } else if self.remaining == 0 {
                Some("Owned-file measurement reached its entry limit")
            } else {
                None
            };
            if let Some(stopped) = stopped {
                complete = false;
                note = Some(stopped.into());
                break;
            }
            self.remaining -= 1;
            let metadata = match path.symlink_metadata() {
                Ok(metadata) => metadata,
                Err(_) => {
                    complete = false;
                    note = Some("Some owned files could not be read".into());
                    continue;
                }
            };
            if metadata.is_dir() {
                continue;
            }
            known = true;
            hard_links |= metadata.is_file() && metadata.nlink() > 1;
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
            if index == 0
                || (index % 256 == 0 && last_progress.elapsed() >= Duration::from_millis(100))
            {
                progress(Measurement {
                    status: Default::default(),
                    bytes: Some(bytes),
                    complete: false,
                    note: Some(super::MEASURING_NOTE.into()),
                });
                last_progress = Instant::now();
            }
        }
        if hard_links {
            let note = note.get_or_insert_with(String::new);
            if !note.is_empty() {
                note.push_str("; ");
            }
            note.push_str(
                "Hard-linked files counted once; their storage may be retained elsewhere",
            );
        }
        Measurement {
            status: if hard_links {
                super::MeasurementStatus::SharedStorage
            } else {
                super::MeasurementStatus::Size
            },
            bytes: known.then_some(bytes),
            complete: known && complete,
            note,
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
