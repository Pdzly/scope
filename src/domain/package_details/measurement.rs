//! Bounded measurement of one managed directory, without following symlinks.

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

pub(super) const TOOL_TIME_LIMIT: Duration = Duration::from_secs(15);
pub(super) const TOOL_ENTRY_LIMIT: usize = 250_000;

/// Coverage of a byte measurement, or why a row has no independent size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeasurementStatus {
    #[default]
    Size,
    SharedStorage,
    Included,
    NotMeasured,
    Unattributed,
}

/// Allocated filesystem bytes, which are an estimate rather than freed space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Measurement {
    #[serde(default)]
    pub status: MeasurementStatus,
    pub bytes: Option<u64>,
    pub complete: bool,
    pub note: Option<String>,
}

impl Measurement {
    pub(crate) fn unknown(note: &str) -> Self {
        Self {
            status: MeasurementStatus::Size,
            bytes: None,
            complete: false,
            note: Some(note.into()),
        }
    }

    pub(crate) fn descriptive(status: MeasurementStatus, note: &str) -> Self {
        Self {
            status,
            ..Self::unknown(note)
        }
    }
}

pub(crate) fn measure(path: &Path, cancel: &AtomicBool) -> Measurement {
    measure_with_limits(path, cancel, Duration::from_secs(2), 20_000)
}

/// Large dedicated tool trees get a bounded budget and publish intermediate totals.
pub(super) fn measure_tool(
    path: &Path,
    cancel: &AtomicBool,
    progress: impl FnMut(Measurement),
) -> Measurement {
    measure_progressive(path, cancel, TOOL_TIME_LIMIT, TOOL_ENTRY_LIMIT, progress)
}

pub(super) fn file_size(path: &Path) -> Measurement {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Measurement {
            status: Default::default(),
            bytes: Some(metadata.len()),
            complete: true,
            note: None,
        },
        Ok(_) => Measurement::unknown("Not a regular file"),
        Err(_) => Measurement::unknown("File could not be read"),
    }
}

fn measure_with_limits(
    path: &Path,
    cancel: &AtomicBool,
    limit: Duration,
    entries: usize,
) -> Measurement {
    measure_progressive(path, cancel, limit, entries, |_| {})
}

fn measure_progressive(
    path: &Path,
    cancel: &AtomicBool,
    limit: Duration,
    entries: usize,
    mut progress: impl FnMut(Measurement),
) -> Measurement {
    let root = match path.symlink_metadata() {
        Ok(root) if root.is_dir() && !root.file_type().is_symlink() => root,
        Ok(_) => return Measurement::unknown("Not a regular managed directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Measurement::unknown("Directory not present")
        }
        Err(_) => return Measurement::unknown("Directory could not be read"),
    };
    let deadline = Instant::now() + limit;
    let mut seen = HashSet::new();
    let mut bytes = 0_u64;
    let mut complete = true;
    let mut note = Some("Disk usage estimate".into());
    let mut hard_links = false;
    let mut last_progress = Instant::now();
    for (count, entry) in WalkDir::new(path)
        .follow_links(false)
        .follow_root_links(false)
        .same_file_system(true)
        .into_iter()
        .enumerate()
    {
        let stopped = if cancel.load(Ordering::Relaxed) {
            Some("Inspection cancelled before all files were measured".to_owned())
        } else if Instant::now() >= deadline {
            Some(format!(
                "Measurement reached its {}-second time limit",
                limit.as_secs()
            ))
        } else if count >= entries {
            Some(format!("Measurement reached its {entries}-entry limit"))
        } else {
            None
        };
        if let Some(stopped) = stopped {
            complete = false;
            note = Some(stopped);
            break;
        }
        let metadata = match entry.and_then(|entry| entry.metadata()) {
            Ok(metadata) => metadata,
            Err(_) => {
                complete = false;
                note = Some("Some files could not be read".into());
                continue;
            }
        };
        if metadata.dev() != root.dev() {
            complete = false;
            note = Some("Other filesystems were not measured".into());
            continue;
        }
        if metadata.file_type().is_symlink() && complete {
            note = Some("Disk usage estimate; link targets excluded".into());
        }
        hard_links |= metadata.is_file() && metadata.nlink() > 1;
        if seen.insert((metadata.dev(), metadata.ino())) {
            let Some(total) = metadata
                .blocks()
                .checked_mul(512)
                .and_then(|size| bytes.checked_add(size))
            else {
                return Measurement::unknown("Directory size exceeded the measurement limit");
            };
            bytes = total;
        }
        if count == 0 || (count % 256 == 0 && last_progress.elapsed() >= Duration::from_millis(100))
        {
            progress(Measurement {
                status: MeasurementStatus::Size,
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
        note.push_str("hard-linked files counted once; their storage may be retained elsewhere");
    }
    Measurement {
        status: if hard_links {
            MeasurementStatus::SharedStorage
        } else {
            MeasurementStatus::Size
        },
        bytes: Some(bytes),
        complete,
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn counts_hard_links_once() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("data");
        fs::write(&file, vec![1_u8; 8192]).unwrap();
        fs::hard_link(&file, directory.path().join("copy")).unwrap();
        let size = measure(directory.path(), &AtomicBool::new(false));
        let expected = (fs::metadata(&file).unwrap().blocks()
            + fs::metadata(directory.path()).unwrap().blocks())
            * 512;
        assert_eq!(size.bytes, Some(expected));
        assert!(size.complete);
        assert!(size
            .note
            .unwrap()
            .contains("hard-linked files counted once"));
    }

    #[test]
    fn does_not_follow_links_outside_the_directory() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), vec![1_u8; 8192]).unwrap();
        symlink(outside.path(), directory.path().join("link")).unwrap();
        let size = measure(directory.path(), &AtomicBool::new(false));
        let expected = (fs::metadata(directory.path()).unwrap().blocks()
            + fs::symlink_metadata(directory.path().join("link"))
                .unwrap()
                .blocks())
            * 512;
        assert_eq!(size.bytes, Some(expected));
        assert!(size.complete);
    }

    #[test]
    fn root_symlink_is_unknown() {
        let directory = tempfile::tempdir().unwrap();
        symlink(directory.path(), directory.path().join("link")).unwrap();
        assert!(
            measure(&directory.path().join("link"), &AtomicBool::new(false))
                .bytes
                .is_none()
        );
    }

    #[test]
    fn missing_directory_is_not_reported_as_zero() {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            measure(&directory.path().join("missing"), &AtomicBool::new(false))
                .bytes
                .is_none()
        );
    }

    #[test]
    fn entry_budget_and_cancellation_produce_partial_measurements() {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            !measure_with_limits(
                directory.path(),
                &AtomicBool::new(false),
                Duration::from_secs(1),
                0
            )
            .complete
        );
        assert!(!measure(directory.path(), &AtomicBool::new(true)).complete);
    }

    #[test]
    fn tool_progress_reports_measured_bytes_and_cancellation_stays_partial() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("data"), vec![1_u8; 8192]).unwrap();
        let cancel = AtomicBool::new(false);
        let mut updates = Vec::new();
        let size = measure_tool(directory.path(), &cancel, |update| {
            updates.push(update);
            cancel.store(true, Ordering::Relaxed);
        });
        assert_eq!(updates.len(), 1);
        assert!(updates[0].bytes.is_some());
        assert!(!updates[0].complete);
        assert!(!size.complete);
        assert!(size.note.unwrap().contains("cancelled"));
    }

    #[test]
    fn limit_notes_identify_time_and_entry_limits() {
        let directory = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        assert!(
            measure_with_limits(directory.path(), &cancel, Duration::ZERO, 10)
                .note
                .unwrap()
                .contains("time limit")
        );
        assert!(
            measure_with_limits(directory.path(), &cancel, Duration::from_secs(1), 0)
                .note
                .unwrap()
                .contains("entry limit")
        );
    }

    #[test]
    fn old_measurements_default_to_byte_size_status() {
        let size: Measurement =
            serde_json::from_str(r#"{"bytes":123,"complete":true,"note":null}"#).unwrap();
        assert_eq!(size.status, MeasurementStatus::Size);
    }
}
