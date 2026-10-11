use super::*;
use crate::domain::tool_managers::python_updates::pipx_candidate;
#[test]
fn registry_candidates_exclude_yanked_prerelease_and_out_of_range_versions() {
    let output = r#"{"versions":[{"num":"1.1.0","yanked":false},{"num":"1.2.0","yanked":true},{"num":"2.0.0","yanked":false},{"num":"1.3.0-beta.1","yanked":false}]}"#;
    assert_eq!(
        registry_candidate(output, "1.0.0", Some("^1"))
            .unwrap()
            .as_deref(),
        Some("1.1.0")
    );
    assert_eq!(
        registry_candidate(output, "1.1.0", Some("=1.1.0")).unwrap(),
        None
    );
    assert!(registry_candidate("{}", "1.0.0", None).is_err());
}
#[test]
fn pipx_update_matches_normalized_distribution_name() {
    let output =
        r#"[{"name":"demo_tool","latest_version":"2.0"},{"name":"other","latest_version":"5.0"}]"#;
    assert_eq!(
        pipx_candidate(output, "demo-tool").unwrap().as_deref(),
        Some("2.0")
    );
    assert_eq!(pipx_candidate(output, "missing").unwrap(), None);
    assert!(pipx_candidate("not json", "demo").is_err());
}
#[test]
fn git_source_keeps_branch_and_detects_pins_without_exposing_passwords() {
    let (url, reference, pinned) =
        git_source("git+https://example.com/repo?branch=main#abc").unwrap();
    assert_eq!(url, "https://example.com/repo");
    assert_eq!(reference, "refs/heads/main");
    assert!(!pinned);
    assert!(
        git_source("git+https://example.com/repo?rev=abc#abc")
            .unwrap()
            .2
    );
    assert!(git_source("git+https://user:secret@example.com/repo#abc").is_err());
}
