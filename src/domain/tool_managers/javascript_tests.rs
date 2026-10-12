use super::super::{javascript_config::node_path, javascript_updates, operations, validate};
use super::*;
use crate::domain::{
    listing,
    operations::{Operation, OperationTarget},
};
use serde_json::json;
use std::{collections::BTreeMap, os::unix::fs::symlink};

fn fixture(source: PackageSource) -> (tempfile::TempDir, GlobalContext) {
    let directory = tempfile::tempdir_in(home().unwrap()).unwrap();
    let root = directory.path().join("global");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let node = (source == PackageSource::Npm).then(|| PathBuf::from("/usr/bin/true"));
    let env = if let Some(node) = &node {
        BTreeMap::from([
            ("NPM_CONFIG_PREFIX".into(), root.display().to_string()),
            ("PATH".into(), node_path(node).unwrap()),
        ])
    } else {
        BTreeMap::from([
            ("BUN_INSTALL_GLOBAL_DIR".into(), root.display().to_string()),
            ("BUN_INSTALL_BIN".into(), bin.display().to_string()),
        ])
    };
    (
        directory,
        GlobalContext {
            root,
            bin,
            node,
            env,
            config_hash: "settings".into(),
        },
    )
}

fn install(source: PackageSource, context: &GlobalContext, name: &str, bin: Value) {
    let environment = modules(source, &context.root).join(name);
    std::fs::create_dir_all(environment.join("bin")).unwrap();
    std::fs::write(environment.join("bin/main.js"), "#!/usr/bin/env node\n").unwrap();
    let manifest = json!({"name": name, "version": "1.0.0", "bin": bin});
    std::fs::write(environment.join("package.json"), manifest.to_string()).unwrap();
    for (name, target) in commands(name, &manifest).unwrap() {
        symlink(environment.join(target), context.bin.join(name)).unwrap();
    }
}

async fn package_fixture(source: PackageSource) -> (tempfile::TempDir, InstalledPackage) {
    let (directory, context) = fixture(source);
    install(
        source,
        &context,
        "@scope/tool",
        json!({"tool": "bin/main.js", "tool-helper": "bin/main.js"}),
    );
    let receipt = if source == PackageSource::Npm {
        json!({"dependencies": {"@scope/tool": {"version": "1.0.0"}}})
    } else {
        json!({"dependencies": {"@scope/tool": "^1.0.0"}})
    };
    let package = read_inventory(source, PathBuf::from("/usr/bin/true"), context, &receipt)
        .await
        .unwrap()
        .remove(0);
    (directory, package)
}

#[test]
fn scoped_names_are_valid_only_for_javascript_packages() {
    assert!(validate_package_name(PackageSource::Npm, "@scope/tool").is_ok());
    assert!(validate_package_name(PackageSource::Bun, "@scope/tool").is_ok());
    assert!(validate_package_name(PackageSource::Cargo, "@scope/tool").is_err());
    for id in [
        "@scope/../tool",
        "@../tool",
        "@scope/--all",
        "@scope/tool/other",
        "@scope/",
        "--global",
        "@scope/tool\n",
    ] {
        assert!(
            validate_package_name(PackageSource::Npm, id).is_err(),
            "{id}"
        );
    }
}

#[test]
fn global_root_identity_tracks_the_target_of_directory_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let one = directory.path().join("one");
    let two = directory.path().join("two");
    std::fs::create_dir(&one).unwrap();
    std::fs::create_dir(&two).unwrap();
    let alias = directory.path().join("alias");
    symlink(&one, &alias).unwrap();
    let first = super::super::javascript_config::canonical_directory(alias.clone()).unwrap();
    std::fs::remove_file(&alias).unwrap();
    symlink(&two, &alias).unwrap();
    let second = super::super::javascript_config::canonical_directory(alias).unwrap();
    assert_ne!(first, second);
}

#[tokio::test]
async fn global_packages_are_one_row_with_all_owned_commands_and_distinct_roots() {
    for source in [PackageSource::Npm, PackageSource::Bun] {
        let (_directory, package) = package_fixture(source).await;
        let (_second_directory, second) = package_fixture(source).await;
        assert_ne!(package.key, second.key);
        assert_eq!(
            package
                .tool_installation
                .as_ref()
                .unwrap()
                .executables
                .len(),
            2
        );
        assert!(listing::is_listable(&package));
        assert!(matches!(
            OperationTarget::from_package(&package).unwrap(),
            OperationTarget::Tool { .. }
        ));
    }
}

#[tokio::test]
async fn npm_commands_always_use_global_mode_and_the_recorded_prefix() {
    let (_directory, package) = package_fixture(PackageSource::Npm).await;
    let installation = package.tool_installation.as_ref().unwrap();
    for (operation, verb) in [
        (Operation::Uninstall, "uninstall"),
        (Operation::Update, "update"),
    ] {
        let command = operations::command("@scope/tool", installation, operation, "1.1.0").unwrap();
        assert_eq!(
            command.args,
            [
                verb,
                "--global",
                "--prefix",
                installation.root.to_str().unwrap(),
                "@scope/tool"
            ]
        );
    }
    assert!(operations::command("other", installation, Operation::Uninstall, "").is_err());
}

#[tokio::test]
async fn bun_commands_preserve_constraints_and_never_request_latest_or_trust() {
    let (_directory, package) = package_fixture(PackageSource::Bun).await;
    let installation = package.tool_installation.as_ref().unwrap();
    assert_eq!(
        operations::command("@scope/tool", installation, Operation::Update, "1.1.0")
            .unwrap()
            .args,
        ["update", "--global", "@scope/tool"]
    );
    assert_eq!(
        operations::command("@scope/tool", installation, Operation::Uninstall, "")
            .unwrap()
            .args,
        ["remove", "--global", "@scope/tool"]
    );
}

#[tokio::test]
async fn conflicting_and_missing_launchers_fail_closed() {
    let (_directory, package) = package_fixture(PackageSource::Npm).await;
    let installation = package.tool_installation.as_ref().unwrap();
    let executable = &installation.executables[0];
    std::fs::remove_file(executable).unwrap();
    assert!(validate(installation).is_err());
    symlink("/usr/bin/true", executable).unwrap();
    assert!(validate(installation).is_err());
    assert!(operations::command("@scope/tool", installation, Operation::Uninstall, "").is_err());
}

#[tokio::test]
async fn local_roots_and_modified_recorded_environments_are_rejected() {
    for source in [PackageSource::Npm, PackageSource::Bun] {
        let (_directory, package) = package_fixture(source).await;
        let mut installation = *package.tool_installation.unwrap();
        installation
            .env
            .insert("LD_PRELOAD".into(), "malicious.so".into());
        assert!(validate(&installation).is_err());
        installation.env.remove("LD_PRELOAD");
        installation.root = home().unwrap();
        assert!(validate(&installation).is_err());
    }
}

#[tokio::test]
async fn symlinked_scope_directories_cannot_claim_another_package() {
    let (_directory, package) = package_fixture(PackageSource::Bun).await;
    let installation = package.tool_installation.as_ref().unwrap();
    let scope = modules(PackageSource::Bun, &installation.root).join("@scope");
    let moved = installation.root.join("other-scope");
    std::fs::rename(&scope, &moved).unwrap();
    symlink(&moved, &scope).unwrap();
    assert!(validate(installation).is_err());
}

#[tokio::test]
async fn npm_extraneous_packages_and_project_dependencies_are_not_inventory() {
    let (_directory, context) = fixture(PackageSource::Npm);
    install(PackageSource::Npm, &context, "tool", json!("bin/main.js"));
    std::fs::write(
        context.root.join("package.json"),
        json!({"dependencies":{"project-only":"*"}}).to_string(),
    )
    .unwrap();
    let receipt =
        json!({"dependencies":{"tool":{"version":"1.0.0"},"leftover":{"extraneous":true}}});
    let packages = read_inventory(
        PackageSource::Npm,
        PathBuf::from("/usr/bin/true"),
        context,
        &receipt,
    )
    .await
    .unwrap();
    assert_eq!(
        packages
            .iter()
            .map(|p| p.package_id.as_str())
            .collect::<Vec<_>>(),
        ["tool"]
    );
}

#[tokio::test]
async fn changed_manifest_or_shared_receipt_invalidates_installation_identity() {
    let (_directory, context) = fixture(PackageSource::Bun);
    install(PackageSource::Bun, &context, "tool", json!("bin/main.js"));
    let receipt = json!({"dependencies":{"tool":"^1.0.0"}});
    let reload = || GlobalContext {
        root: context.root.clone(),
        bin: context.bin.clone(),
        node: None,
        env: context.env.clone(),
        config_hash: context.config_hash.clone(),
    };
    let first = read_inventory(
        PackageSource::Bun,
        PathBuf::from("/usr/bin/true"),
        reload(),
        &receipt,
    )
    .await
    .unwrap();
    std::fs::write(context.root.join("bun.lock"), "changed shared lock").unwrap();
    let second = read_inventory(
        PackageSource::Bun,
        PathBuf::from("/usr/bin/true"),
        reload(),
        &receipt,
    )
    .await
    .unwrap();
    assert_ne!(first[0].tool_installation, second[0].tool_installation);
    let path = modules(PackageSource::Bun, &context.root).join("tool/package.json");
    let mut manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    manifest["version"] = json!("1.1.0");
    std::fs::write(path, manifest.to_string()).unwrap();
    let third = read_inventory(
        PackageSource::Bun,
        PathBuf::from("/usr/bin/true"),
        reload(),
        &receipt,
    )
    .await
    .unwrap();
    assert_ne!(second[0].tool_installation, third[0].tool_installation);
}

#[tokio::test]
async fn local_bun_specs_disable_update_but_allow_owned_package_uninstall() {
    let (_directory, mut package) = package_fixture(PackageSource::Bun).await;
    let installation = package.tool_installation.as_mut().unwrap();
    if let UpdateRecipe::Javascript {
        registry_source, ..
    } = &mut installation.update
    {
        *registry_source = false;
    }
    assert!(operations::command("@scope/tool", installation, Operation::Update, "1.1.0").is_err());
    assert!(operations::command("@scope/tool", installation, Operation::Uninstall, "").is_ok());
}

#[test]
fn manager_packages_are_protected_without_relying_on_the_ui() {
    for source in [PackageSource::Npm, PackageSource::Bun] {
        for id in ["npm", "bun", "node", "corepack", "scope-gpui"] {
            assert!(crate::domain::safety::check_package(source, id).protected);
        }
        assert!(!crate::domain::safety::check_package(source, "@scope/tool").protected);
    }
}

#[test]
fn command_targets_cannot_escape_package_directories() {
    for target in ["../other.js", "/usr/bin/node", "bin/../../other.js"] {
        assert!(commands("tool", &json!({"bin":{"tool":target}})).is_err());
    }
}

#[test]
fn npm_update_errors_and_stale_versions_are_distinct_from_up_to_date() {
    assert!(javascript_updates::npm_candidate(
        r#"{"error":{"code":"ENETUNREACH"}}"#,
        "tool",
        "1.0.0"
    )
    .is_err());
    assert!(javascript_updates::npm_candidate("garbage", "tool", "1.0.0").is_err());
    assert!(javascript_updates::npm_candidate(
        r#"{"tool":{"current":"0.9.0","wanted":"1.1.0"}}"#,
        "tool",
        "1.0.0"
    )
    .is_err());
    assert_eq!(
        javascript_updates::npm_candidate("{}", "tool", "1.0.0").unwrap(),
        None
    );
    assert_eq!(
        javascript_updates::npm_candidate(
            r#"{"tool":{"current":"1.0.0","wanted":"1.1.0","latest":"2.0.0"}}"#,
            "tool",
            "1.0.0"
        )
        .unwrap(),
        Some("1.1.0".into())
    );
}

#[test]
fn bun_uses_native_range_resolution_and_rejects_unavailable_responses() {
    assert_eq!(
        javascript_updates::bun_candidate(r#"["1.0.0","1.2.0","1.1.0"]"#, "1.0.0").unwrap(),
        Some("1.2.0".into())
    );
    assert_eq!(
        javascript_updates::bun_candidate(r#""1.0.0""#, "1.0.0").unwrap(),
        None
    );
    assert!(javascript_updates::bun_candidate("{}", "1.0.0").is_err());
    for spec in [
        "file:../tool",
        "git+https://host/tool",
        "npm:other@^1",
        "https://host/tool.tgz",
    ] {
        assert!(!registry_requirement(spec));
    }
}

#[test]
fn old_cached_availability_defaults_new_managers_to_unavailable() {
    let availability: crate::domain::scanner::ScanAvailability = serde_json::from_value(
        json!({"apt":false,"snap":false,"flatpak":false,"appimage":false,"appimage_dirs":[]}),
    )
    .unwrap();
    assert!(!availability.npm && !availability.bun);
}

#[tokio::test]
async fn npm_json_completion_order_does_not_invalidate_a_preview() {
    let (_directory, context) = fixture(PackageSource::Npm);
    install(PackageSource::Npm, &context, "tool", json!("bin/main.js"));
    let first: Value = serde_json::from_str(
        r#"{"name":"lib","dependencies":{"tool":{"version":"1.0.0","extraneous":false}}}"#,
    )
    .unwrap();
    let second: Value = serde_json::from_str(
        r#"{"dependencies":{"tool":{"extraneous":false,"version":"1.0.0"}},"name":"lib"}"#,
    )
    .unwrap();
    let reload = || GlobalContext {
        root: context.root.clone(),
        bin: context.bin.clone(),
        node: context.node.clone(),
        env: context.env.clone(),
        config_hash: context.config_hash.clone(),
    };
    let one = read_inventory(
        PackageSource::Npm,
        PathBuf::from("/usr/bin/true"),
        reload(),
        &first,
    )
    .await
    .unwrap();
    let two = read_inventory(
        PackageSource::Npm,
        PathBuf::from("/usr/bin/true"),
        reload(),
        &second,
    )
    .await
    .unwrap();
    assert_eq!(one[0].tool_installation, two[0].tool_installation);
}

#[tokio::test]
async fn aliased_global_packages_allow_uninstall_but_do_not_change_registry_identity() {
    let (_directory, context) = fixture(PackageSource::Bun);
    install(
        PackageSource::Bun,
        &context,
        "alias",
        json!({"tool":"bin/main.js"}),
    );
    let manifest = modules(PackageSource::Bun, &context.root).join("alias/package.json");
    std::fs::write(
        &manifest,
        json!({"name":"@real/tool","version":"1.0.0","bin":{"tool":"bin/main.js"}}).to_string(),
    )
    .unwrap();
    let receipt = json!({"dependencies":{"alias":"npm:@real/tool@^1"}});
    let package = read_inventory(
        PackageSource::Bun,
        PathBuf::from("/usr/bin/true"),
        context,
        &receipt,
    )
    .await
    .unwrap()
    .remove(0);
    let installation = package.tool_installation.as_ref().unwrap();
    assert!(operations::command("alias", installation, Operation::Uninstall, "").is_ok());
    assert!(operations::command("alias", installation, Operation::Update, "1.1.0").is_err());
    std::fs::write(
        &manifest,
        json!({"name":"npm","version":"1.0.0","bin":{"tool":"bin/main.js"}}).to_string(),
    )
    .unwrap();
    assert!(validate(installation).is_err());
}

#[tokio::test]
async fn intentionally_protected_managers_do_not_report_a_failed_update_check() {
    let mut package = InstalledPackage::new(PackageSource::Npm, "npm");
    assert_eq!(
        super::super::updates::check(std::slice::from_mut(&mut package)).await,
        None
    );
    assert!(package
        .update_note
        .as_deref()
        .unwrap()
        .contains("Package managers"));
    assert!(!package.has_update);
}

#[tokio::test]
async fn javascript_inspection_never_measures_the_shared_modules_root() {
    use crate::domain::package_details::{self, DetailEffect};
    let (_directory, package) = package_fixture(PackageSource::Bun).await;
    let installation = package.tool_installation.as_ref().unwrap();
    let neighbor = modules(PackageSource::Bun, &installation.root).join("shared-dependency");
    std::fs::create_dir_all(&neighbor).unwrap();
    std::fs::write(neighbor.join("data"), vec![0_u8; 1024 * 1024]).unwrap();
    let details = package_details::inspect_progressive(
        package.clone(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        |_| {},
    )
    .await;
    assert!(details.warnings.is_empty(), "{:?}", details.warnings);
    let removed = details
        .items
        .iter()
        .filter(|item| item.effect == DetailEffect::Removed)
        .collect::<Vec<_>>();
    assert_eq!(removed.len(), 1);
    assert_eq!(
        removed[0].location.as_deref(),
        installation.environment.as_ref().unwrap().to_str()
    );
    assert!(removed[0].size.bytes.unwrap() < 1024 * 1024);
    assert!(details
        .items
        .iter()
        .any(|item| item.label == "Shared dependencies"
            && item.effect == DetailEffect::Informational
            && item.size.bytes.is_none()));
}
