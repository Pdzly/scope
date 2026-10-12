use super::*;
use crate::domain::{
    package::{AppKind, PackageSource},
    tool_managers::{ToolInstallation, UpdateRecipe},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

fn fixture(source: PackageSource) -> (tempfile::TempDir, InstalledPackage) {
    let directory = tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
    let root = directory.path().join("tools");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let environment = match source {
        PackageSource::Cargo => None,
        PackageSource::Npm => Some(root.join("lib/node_modules/demo")),
        PackageSource::Bun => Some(root.join("node_modules/demo")),
        _ => Some(root.join("demo")),
    };
    let executable = bin.join("demo");
    if let Some(environment) = &environment {
        std::fs::create_dir_all(environment.join("bin")).unwrap();
        std::fs::write(environment.join("bin/demo"), vec![1_u8; 8192]).unwrap();
        std::os::unix::fs::symlink(environment.join("bin/demo"), &executable).unwrap();
        std::fs::write(
            environment.join("package.json"),
            r#"{"name":"demo","version":"1.0.0","bin":{"demo":"bin/demo"}}"#,
        )
        .unwrap();
    } else {
        std::fs::write(&executable, vec![1_u8; 8192]).unwrap();
    }
    let mut env = BTreeMap::new();
    let update = if matches!(source, PackageSource::Npm | PackageSource::Bun) {
        let node = (source == PackageSource::Npm).then(|| PathBuf::from("/usr/bin/true"));
        if node.is_some() {
            env.insert("NPM_CONFIG_PREFIX".into(), root.display().to_string());
            env.insert(
                "PATH".into(),
                format!("/usr/bin:{}", std::env::var("PATH").unwrap()),
            );
        } else {
            env.insert("BUN_INSTALL_GLOBAL_DIR".into(), root.display().to_string());
            env.insert("BUN_INSTALL_BIN".into(), bin.display().to_string());
        }
        UpdateRecipe::Javascript {
            package_name: "demo".into(),
            bin,
            node,
            requirement: "*".into(),
            registry_source: true,
        }
    } else {
        UpdateRecipe::Unsupported("fixture".into())
    };
    let mut package = InstalledPackage::new(source, "demo");
    package.app_kind = AppKind::Cli;
    package.tool_installation = Some(Box::new(ToolInstallation {
        source,
        manager: "/usr/bin/true".into(),
        root,
        environment,
        executables: vec![executable],
        env,
        receipt_hash: "fixture".into(),
        protection_reason: None,
        update,
        notes: vec![],
    }));
    (directory, package)
}

#[test]
fn large_tool_environments_are_not_cut_off_at_twenty_thousand_entries() {
    let (_directory, package) = fixture(PackageSource::Uv);
    let environment = package
        .tool_installation
        .as_ref()
        .unwrap()
        .environment
        .as_ref()
        .unwrap();
    let files = environment.join("files");
    std::fs::create_dir(&files).unwrap();
    for index in 0..20_010 {
        std::fs::write(files.join(index.to_string()), []).unwrap();
    }
    let publish: super::super::inspection::Publish = Arc::new(|_| {});
    let details = super::super::targets::measure(
        discover(&package).unwrap(),
        &AtomicBool::new(false),
        &publish,
    );
    assert!(
        details.items[0].size.complete,
        "{:?}",
        details.items[0].size.note
    );
}

#[test]
fn shared_tool_data_is_explicitly_kept_for_all_five_managers() {
    for source in [
        PackageSource::Uv,
        PackageSource::Pipx,
        PackageSource::Cargo,
        PackageSource::Npm,
        PackageSource::Bun,
    ] {
        let (_directory, package) = fixture(source);
        let details = discover(&package).unwrap().details;
        let shared = details
            .items
            .iter()
            .find(|item| item.label == "Shared data")
            .unwrap();
        assert_eq!(shared.effect, DetailEffect::Kept, "{source:?}");
    }
}

#[test]
fn all_tool_sources_measure_only_owned_footprints_and_classify_non_sizes() {
    use super::super::MeasurementStatus;
    for source in [
        PackageSource::Uv,
        PackageSource::Pipx,
        PackageSource::Cargo,
        PackageSource::Npm,
        PackageSource::Bun,
    ] {
        let (_directory, mut package) = fixture(source);
        let installation = package.tool_installation.as_mut().unwrap();
        std::fs::write(
            installation.root.join("bin/unowned"),
            vec![1_u8; 1024 * 1024],
        )
        .unwrap();
        if source == PackageSource::Cargo {
            installation
                .executables
                .push(installation.executables[0].clone());
        }
        let updates = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = updates.clone();
        let publish: super::super::inspection::Publish = Arc::new(move |update| {
            received.lock().unwrap().push(update);
        });
        let details = super::super::targets::measure(
            discover(&package).unwrap(),
            &AtomicBool::new(false),
            &publish,
        );
        let size = details.removal_size();
        assert!(
            size.complete,
            "{source:?}: {:?}",
            details.items[0].size.note
        );
        assert!(
            size.bytes.unwrap() > 0 && size.bytes.unwrap() < 1024 * 1024,
            "{source:?}"
        );
        assert!(
            updates
                .lock()
                .unwrap()
                .iter()
                .any(|update| update.details.items[0].size.bytes.is_some()
                    && !update.details.items[0].size.complete),
            "{source:?}: no byte progress"
        );
        for item in details.items.iter().filter(|item| item.label == "Command") {
            assert_eq!(item.size.status, MeasurementStatus::Included);
            assert_eq!(item.effect, DetailEffect::Informational);
        }
        let user = details
            .items
            .iter()
            .find(|item| item.label == "User data")
            .unwrap();
        assert_eq!(user.size.status, MeasurementStatus::Unattributed);
    }
}

#[tokio::test]
#[ignore = "reads one installed tool per available source without launching apps or changing installations"]
async fn live_tool_measurements_cover_installed_sources() {
    for source in [
        PackageSource::Uv,
        PackageSource::Pipx,
        PackageSource::Cargo,
        PackageSource::Npm,
        PackageSource::Bun,
    ] {
        if !crate::domain::system::which(crate::domain::tool_managers::manager_name(source)) {
            continue;
        }
        let packages = crate::domain::tool_managers::inventory(source)
            .await
            .unwrap();
        let package = packages
            .iter()
            .find(|package| {
                package.package_id == "command-code"
                    && crate::domain::listing::can_uninstall(package)
            })
            .or_else(|| {
                packages
                    .iter()
                    .find(|package| crate::domain::listing::can_uninstall(package))
            });
        let Some(package) = package else {
            eprintln!("{source:?}: no managed global tools to measure");
            continue;
        };
        let start = std::time::Instant::now();
        let details = crate::domain::package_details::inspect_progressive(
            package.clone(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await;
        assert!(
            details.warnings.is_empty(),
            "{source:?}: {:?}",
            details.warnings
        );
        let size = details.removal_size();
        assert!(size.bytes.is_some(), "{source:?}: no attributable size");
        eprintln!(
            "{source:?} {}: {} allocated bytes, complete={}, {:.2}s",
            package.package_id,
            size.bytes.unwrap(),
            size.complete,
            start.elapsed().as_secs_f32()
        );
    }
}
