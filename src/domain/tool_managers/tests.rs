//! Fixtures exercise manager identity, parsing, and fail-closed operations.
use super::*;
use crate::domain::operations::{Operation, OperationTarget};

pub(super) fn fixture(source: PackageSource) -> (tempfile::TempDir, ToolInstallation) {
    let dir = tempfile::tempdir_in(home().unwrap()).unwrap();
    let root = dir.path().join("tools");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let environment = (source != PackageSource::Cargo).then(|| root.join("demo"));
    if let Some(env) = &environment {
        std::fs::create_dir_all(env.join("bin")).unwrap();
    }
    let installation = ToolInstallation {
        source,
        manager: PathBuf::from("/usr/bin/true"),
        root,
        environment,
        executables: vec![],
        env: BTreeMap::new(),
        receipt_hash: "receipt".into(),
        protection_reason: None,
        update: UpdateRecipe::Uv {
            requirements: vec!["demo".into()],
            constraints: vec![],
            overrides: vec![],
            resolver_config: None,
        },
        notes: vec![],
    };
    (dir, installation)
}

pub(super) fn cargo_receipt(version: &str, features: &str) -> String {
    format!(
        r#"{{"installs":{{"demo {version} (registry+https://github.com/rust-lang/crates.io-index)":{{"bins":["demo","demo-helper"],"features":["{features}"],"all_features":false,"no_default_features":true,"profile":"release","target":"x86_64-unknown-linux-gnu","version_req":"^1"}}}}}}"#
    )
}

#[test]
fn rejects_option_names_and_path_traversal() {
    for name in [
        "",
        "--all",
        "../demo",
        "demo/other",
        "demo\nother",
        "demo;rm",
        ".demo",
    ] {
        assert!(validate_name(name).is_err(), "{name}");
    }
    assert!(validate_name("demo-tool_2.0").is_ok());
}

#[test]
fn cached_environment_cannot_inject_process_settings_or_change_the_root() {
    let (_dir, mut installation) = fixture(PackageSource::Uv);
    installation
        .env
        .insert("LD_PRELOAD".into(), "/tmp/library.so".into());
    assert!(validate(&installation).is_err());
    installation.env.clear();
    installation
        .env
        .insert("UV_TOOL_DIR".into(), "/tmp/other-tools".into());
    assert!(validate(&installation).is_err());
}

#[test]
fn roots_and_sources_distinguish_installations() {
    let (_dir, first) = fixture(PackageSource::Uv);
    let (_dir2, mut second) = fixture(PackageSource::Uv);
    let one = package("demo", "1.0", first.clone()).unwrap();
    assert_ne!(one.key, package("demo", "1.0", second.clone()).unwrap().key);
    second.root = first.root;
    second.source = PackageSource::Pipx;
    assert_ne!(one.key, package("demo", "1.0", second).unwrap().key);
}

#[test]
fn old_cached_package_cannot_authorize_a_tool_operation() {
    let pkg = InstalledPackage::new(PackageSource::Uv, "demo");
    assert!(OperationTarget::from_package(&pkg).is_err());
    assert!(!crate::domain::listing::can_uninstall(&pkg));
}

#[test]
fn cargo_receipt_preserves_binary_selection_and_build_settings() {
    let parsed = cargo::parse(
        &cargo_receipt("1.0.0", "fast"),
        "/usr/bin/true".into(),
        "/home/test/cargo".into(),
        BTreeMap::new(),
    )
    .unwrap();
    let install = parsed[0].tool_installation.as_ref().unwrap();
    let UpdateRecipe::Cargo {
        options,
        version_req,
        ..
    } = &install.update
    else {
        panic!("Cargo recipe expected");
    };
    assert!(options.windows(2).any(|p| p == ["--bin", "demo-helper"]));
    assert!(options.windows(2).any(|p| p == ["--features", "fast"]));
    assert!(options.contains(&"--no-default-features".into()));
    assert_eq!(version_req.as_deref(), Some("^1"));
}

#[test]
fn corrupt_cargo_receipts_and_unsafe_binary_names_fail_closed() {
    assert!(cargo::parse(
        "{}",
        "/bin/true".into(),
        "/home/test".into(),
        BTreeMap::new()
    )
    .is_err());
    let raw = cargo_receipt("1.0.0", "fast").replace("demo-helper", "../victim");
    assert!(cargo::parse(
        &raw,
        "/bin/true".into(),
        "/home/test".into(),
        BTreeMap::new()
    )
    .is_err());
}

#[tokio::test]
async fn changed_receipt_invalidates_an_existing_preview() {
    let (_dir, seed) = fixture(PackageSource::Cargo);
    std::fs::write(seed.root.join(".crates.toml"), "[v1]\n\"demo 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)\" = [\"demo\", \"demo-helper\"]\n").unwrap();
    let path = seed.root.join(".crates2.json");
    std::fs::write(&path, cargo_receipt("1.0.0", "fast")).unwrap();
    let packages = inventory_recorded(&seed).await.unwrap();
    let install = packages[0].tool_installation.as_ref().unwrap();
    operations::verify_metadata("demo", install, "1.0.0")
        .await
        .unwrap();
    std::fs::write(path, cargo_receipt("1.0.0", "different")).unwrap();
    assert!(operations::verify_metadata("demo", install, "1.0.0")
        .await
        .is_err());
}

#[test]
fn pinned_pipx_upgrade_is_rejected_but_uninstall_remains_available() {
    let (_dir, mut installation) = fixture(PackageSource::Pipx);
    installation.update = UpdateRecipe::Pipx {
        package: "demo".into(),
        pip_args: vec![],
        pinned: true,
        index_source: true,
    };
    assert!(operations::command("demo", &installation, Operation::Update, "2.0").is_err());
    assert!(operations::command("demo", &installation, Operation::Uninstall, "").is_ok());
}

#[test]
fn pipx_preserves_options_as_one_argument_and_redacts_preview_credentials() {
    let (_dir, mut installation) = fixture(PackageSource::Pipx);
    installation.update = UpdateRecipe::Pipx {
        package: "demo".into(),
        pip_args: vec![
            "--index-url".into(),
            "https://user:secret@example.com/simple".into(),
        ],
        pinned: false,
        index_source: true,
    };
    let command = operations::command("demo", &installation, Operation::Update, "2.0").unwrap();
    assert_eq!(command.args.len(), 3);
    assert!(command.args[2].contains("secret"));
    assert!(!command.summary.contains("secret"));
    assert!(!command
        .args
        .iter()
        .any(|arg| arg == "--force" || arg == "--include-injected"));
}

#[test]
fn cargo_command_never_forces_overwrite_and_preserves_original_constraint() {
    let (_dir, seed) = fixture(PackageSource::Cargo);
    let packages = cargo::parse(
        &cargo_receipt("1.0.0", "fast"),
        seed.manager,
        seed.root,
        BTreeMap::new(),
    )
    .unwrap();
    let installation = packages[0].tool_installation.as_ref().unwrap();
    let command = operations::command("demo", installation, Operation::Update, "1.1.0").unwrap();
    assert!(command
        .args
        .windows(2)
        .any(|p| p == ["--registry", "crates-io"]));
    assert!(command.args.windows(2).any(|p| p == ["--version", "^1"]));
    assert!(!command.args.iter().any(|arg| arg == "--force"));
    assert!(operations::command("demo", installation, Operation::Update, "2.0.0").is_err());
}

#[test]
fn command_owned_by_another_environment_is_rejected() {
    use std::os::unix::fs::symlink;
    let (dir, mut installation) = fixture(PackageSource::Uv);
    let victim = dir.path().join("other-app");
    std::fs::write(&victim, "other").unwrap();
    let exposed = installation.root.join("bin/demo");
    symlink(&victim, &exposed).unwrap();
    installation.executables.push(exposed);
    assert!(validate(&installation).is_err());
}

#[test]
fn symlinked_environment_cannot_measure_or_remove_an_unrelated_directory() {
    use std::os::unix::fs::symlink;
    let (dir, mut installation) = fixture(PackageSource::Uv);
    let victim = dir.path().join("personal");
    std::fs::create_dir(&victim).unwrap();
    let environment = installation.root.join("linked");
    symlink(&victim, &environment).unwrap();
    installation.environment = Some(environment);
    assert!(validate(&installation).is_err());
}

#[test]
fn manager_packages_are_protected_independently_of_ui() {
    for source in [PackageSource::Uv, PackageSource::Pipx, PackageSource::Cargo] {
        assert!(crate::domain::safety::check_package(source, "uv").protected);
        assert!(
            operations::command("--all", &fixture(source).1, Operation::Uninstall, "").is_err()
        );
    }
}

#[test]
fn pipx_suffix_and_injected_packages_are_one_environment() {
    let output = r#"{"venvs":{"demo-alt":{"metadata":{"main_package":{"package":"demo","package_version":"1.0","package_or_url":"demo==1.0","suffix":"-alt","apps":["run"],"include_apps":true,"pip_args":[]},"injected_packages":{"plugin":{"package":"plugin","apps":["plugin"],"include_apps":true}}}}}}"#;
    let env = BTreeMap::from([
        ("PIPX_HOME".into(), "/home/test/pipx".into()),
        ("PIPX_BIN_DIR".into(), "/home/test/bin".into()),
    ]);
    let packages = pipx::parse(output, "/bin/true".into(), env).unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].package_id, "demo-alt");
    let installation = packages[0].tool_installation.as_ref().unwrap();
    assert!(installation
        .executables
        .contains(&"/home/test/bin/run-alt".into()));
    assert!(installation
        .notes
        .iter()
        .any(|note| note.contains("plugin")));
}

#[test]
fn uv_lists_packages_instead_of_one_row_per_command() {
    let rows = uv::versions("demo v1.0.0\n- run\n- helper\nother v2.0.0\n- other\n").unwrap();
    assert_eq!(rows.len(), 2);
    assert!(uv::versions("malformed output").is_err());
}

#[tokio::test]
async fn cargo_config_precedence_matches_native_legacy_and_relative_roots() {
    let directory = tempfile::tempdir().unwrap();
    let local = directory.path().join(".cargo");
    let global = directory.path().join("global");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(&global).unwrap();
    std::fs::write(local.join("config"), "[install]\nroot = \"local-tools\"\n").unwrap();
    std::fs::write(local.join("config.toml"), "[install]\nroot = \"ignored\"\n").unwrap();
    std::fs::write(
        global.join("config.toml"),
        "[install]\nroot = \"global-tools\"\n",
    )
    .unwrap();
    assert_eq!(
        cargo::config_root(&[local, global]).await.unwrap(),
        Some(directory.path().join("local-tools"))
    );
}

#[tokio::test]
async fn disagreement_between_cargo_ownership_receipts_fails_closed() {
    let (_directory, context) = fixture(PackageSource::Cargo);
    std::fs::write(
        context.root.join(".crates2.json"),
        cargo_receipt("1.0.0", "fast"),
    )
    .unwrap();
    std::fs::write(context.root.join(".crates.toml"), "[v1]\n\"demo 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)\" = [\"other-command\"]\n").unwrap();
    assert!(inventory_recorded(&context).await.is_err());
}

#[tokio::test]
async fn cached_manager_path_cannot_launch_an_arbitrary_program() {
    let (directory, mut installation) = fixture(PackageSource::Uv);
    let script = directory.path().join("fake-manager");
    let marker = directory.path().join("ran");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    installation.manager = script;
    assert!(operations::verify("demo", &installation, "1.0")
        .await
        .is_err());
    assert!(!marker.exists());
}

#[test]
fn unsupported_python_origins_keep_uninstall_but_reject_update() {
    let (_directory, mut installation) = fixture(PackageSource::Uv);
    installation.update = UpdateRecipe::Unsupported("Git release lookup unavailable".into());
    assert!(operations::command("demo", &installation, Operation::Uninstall, "").is_ok());
    assert!(operations::command("demo", &installation, Operation::Update, "latest").is_err());
}
