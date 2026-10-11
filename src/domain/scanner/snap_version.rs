//! Read-only app version enrichment, separate from Snap's package version.

use std::io::Read;
use std::path::Path;

use serde::Deserialize;

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// VS Code's snap reports a commit hash; its revision contains the app release.
/// Never execute the app or read `current`, which can change during a scan.
pub(super) fn display_version(root: &Path, name: &str, revision: &str) -> Option<String> {
    if name != "code" || revision.is_empty() || !revision.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let path = root
        .join(name)
        .join(revision)
        .join("usr/share/code/resources/app/package.json");
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return None;
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).ok()?;
    let version = manifest.version;
    let core = version.split(['-', '+']).next()?;
    let parts: Vec<_> = core.split('.').collect();
    (version.len() <= 64
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
        && parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())))
    .then_some(version)
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_or_invalid_metadata_does_not_supply_a_display_version() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(display_version(root.path(), "code", "268"), None);
        let path = root
            .path()
            .join("code/268/usr/share/code/resources/app/package.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for bytes in [
            b"not json".as_slice(),
            br#"{"version":"2a59476c"}"#,
            br#"{"version":42}"#,
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(display_version(root.path(), "code", "268"), None);
        }
        std::fs::write(&path, vec![b' '; MAX_MANIFEST_BYTES as usize + 1]).unwrap();
        assert_eq!(display_version(root.path(), "code", "268"), None);
    }

    #[test]
    fn enrichment_uses_the_scanned_revision_instead_of_current() {
        let root = tempfile::tempdir().unwrap();
        let path = root
            .path()
            .join("code/current/usr/share/code/resources/app/package.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, br#"{"version":"9.9.9"}"#).unwrap();
        assert_eq!(display_version(root.path(), "code", "268"), None);
        assert_eq!(display_version(root.path(), "code", "../current"), None);
        assert_eq!(display_version(root.path(), "other", "268"), None);
    }
}
