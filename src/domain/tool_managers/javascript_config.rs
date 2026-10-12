//! Resolve global roots without consulting a project's dependency tree.
use super::{fingerprint, home, query, read};
use crate::domain::{package::PackageSource, system};
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(super) struct GlobalContext {
    pub root: PathBuf,
    pub bin: PathBuf,
    pub node: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub config_hash: String,
}

pub(super) async fn discover(source: PackageSource, manager: &Path) -> Result<GlobalContext> {
    let user_home = home()?;
    let (root, bin, node, env) = match source {
        PackageSource::Npm => {
            let root = canonical_directory(PathBuf::from(
                query(manager, &["prefix", "--global"]).await?.trim(),
            ))?;
            let executable = system::resolve("node").context("Node runtime is unavailable")?;
            let node = PathBuf::from(
                query(&executable, &["-p", "process.execPath"])
                    .await?
                    .trim(),
            );
            if !node.is_absolute() || !node.is_file() {
                bail!("Node runtime could not be verified");
            }
            let env = BTreeMap::from([
                ("NPM_CONFIG_PREFIX".into(), root.display().to_string()),
                ("PATH".into(), node_path(&node)?),
            ]);
            (root.clone(), root.join("bin"), Some(node), env)
        }
        PackageSource::Bun => {
            let (root, bin) = bun_directories(&user_home).await?;
            let root = canonical_directory(root)?;
            let bin = canonical_directory(bin)?;
            let env = BTreeMap::from([
                ("BUN_INSTALL_GLOBAL_DIR".into(), root.display().to_string()),
                ("BUN_INSTALL_BIN".into(), bin.display().to_string()),
            ]);
            (root, bin, None, env)
        }
        _ => bail!("Not a JavaScript tool manager"),
    };
    if !root.is_absolute() || !bin.is_absolute() {
        bail!("Global installation paths must be absolute");
    }
    // Hash settings, including credentials, without exposing their contents in DTOs or previews.
    let mut configs = vec![user_home.join(".npmrc"), root.join(".npmrc")];
    if source == PackageSource::Npm {
        configs.push(root.join("etc/npmrc"));
        if let Some(node) = &node {
            if let Some(prefix) = node.parent().and_then(Path::parent) {
                configs.push(prefix.join("lib/node_modules/npm/npmrc"));
            }
        }
        for key in ["NPM_CONFIG_USERCONFIG", "NPM_CONFIG_GLOBALCONFIG"] {
            if let Some(path) = std::env::var_os(key) {
                configs.push(PathBuf::from(path));
            }
        }
    } else {
        configs.extend(bun_configs(&user_home));
    }
    let mut settings = String::new();
    for path in configs {
        settings.push_str(&path.display().to_string());
        if path.try_exists()? {
            settings.push_str(&read(&path).await?);
        }
    }
    let variables = std::env::vars()
        .filter(|(key, _)| {
            key.to_ascii_lowercase().starts_with("npm_config_") || key.starts_with("BUN_INSTALL")
        })
        .collect::<BTreeMap<_, _>>();
    for (key, value) in variables {
        settings.push_str(&key);
        settings.push_str(&value);
    }
    Ok(GlobalContext {
        root,
        bin,
        node,
        env,
        config_hash: fingerprint(settings.as_bytes()),
    })
}

pub(super) fn canonical_directory(path: PathBuf) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("Global directories must be absolute");
    }
    Ok(if path.try_exists()? {
        path.canonicalize()?
    } else {
        path
    })
}

pub(super) fn node_path(node: &Path) -> Result<String> {
    let parent = node.parent().context("Node runtime has no directory")?;
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let paths = std::iter::once(parent.to_path_buf()).chain(std::env::split_paths(&inherited));
    std::env::join_paths(paths)?
        .into_string()
        .map_err(|_| anyhow::anyhow!("PATH is not UTF-8"))
}

fn bun_configs(user_home: &Path) -> Vec<PathBuf> {
    let global = std::env::var_os("XDG_CONFIG_HOME")
        .map(|path| PathBuf::from(path).join(".bunfig.toml"))
        .unwrap_or_else(|| user_home.join(".bunfig.toml"));
    vec![global, user_home.join("bunfig.toml")]
}

async fn bun_directories(user_home: &Path) -> Result<(PathBuf, PathBuf)> {
    let install = std::env::var_os("BUN_INSTALL")
        .map(PathBuf::from)
        .unwrap_or(user_home.join(".bun"));
    let mut root = install.join("install/global");
    let mut bin = install.join("bin");
    for path in bun_configs(user_home) {
        if path.try_exists()? {
            let config: toml::Value = toml::from_str(&read(&path).await?)?;
            if let Some(directory) = config
                .get("install")
                .and_then(|v| v.get("globalDir"))
                .and_then(toml::Value::as_str)
            {
                root = expand_directory(directory, user_home);
            }
            if let Some(directory) = config
                .get("install")
                .and_then(|value| value.get("globalBinDir"))
                .and_then(toml::Value::as_str)
            {
                bin = expand_directory(directory, user_home);
            }
        }
    }
    if let Some(directory) = std::env::var_os("BUN_INSTALL_GLOBAL_DIR") {
        root = PathBuf::from(directory);
    }
    if let Some(directory) = std::env::var_os("BUN_INSTALL_BIN") {
        bin = PathBuf::from(directory);
    }
    Ok((
        if root.is_absolute() {
            root
        } else {
            user_home.join(root)
        },
        if bin.is_absolute() {
            bin
        } else {
            user_home.join(bin)
        },
    ))
}

pub(super) fn expand_directory(value: &str, user_home: &Path) -> PathBuf {
    value
        .strip_prefix("~/")
        .map(|suffix| user_home.join(suffix))
        .unwrap_or_else(|| PathBuf::from(value))
}
