//! Ownership checks shared by tool plans and footprint inspection.
use super::{home, validate_name, ToolInstallation};
use anyhow::{bail, Context, Result};

/// User ownership is checked before preview, apply, or footprint measurement.
pub fn validate(installation: &ToolInstallation) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let javascript = matches!(
        installation.source,
        crate::domain::package::PackageSource::Npm | crate::domain::package::PackageSource::Bun
    );
    let allowed: &[&str] = match installation.source {
        crate::domain::package::PackageSource::Uv => &["UV_TOOL_DIR", "UV_TOOL_BIN_DIR"],
        crate::domain::package::PackageSource::Pipx => {
            &["PIPX_HOME", "PIPX_BIN_DIR", "PIPX_MAN_DIR"]
        }
        crate::domain::package::PackageSource::Cargo => &["CARGO_HOME", "CARGO_INSTALL_ROOT"],
        crate::domain::package::PackageSource::Npm => &["NPM_CONFIG_PREFIX", "PATH"],
        crate::domain::package::PackageSource::Bun => {
            &["BUN_INSTALL_GLOBAL_DIR", "BUN_INSTALL_BIN"]
        }
        _ => bail!("Not a tool manager installation"),
    };
    if installation
        .env
        .keys()
        .any(|key| !allowed.contains(&key.as_str()))
    {
        bail!("Recorded tool environment contains an unsupported variable");
    }
    let root_key = match installation.source {
        crate::domain::package::PackageSource::Uv => "UV_TOOL_DIR",
        crate::domain::package::PackageSource::Pipx => "PIPX_HOME",
        crate::domain::package::PackageSource::Npm => "NPM_CONFIG_PREFIX",
        crate::domain::package::PackageSource::Bun => "BUN_INSTALL_GLOBAL_DIR",
        _ => "CARGO_INSTALL_ROOT",
    };
    if installation
        .env
        .get(root_key)
        .is_some_and(|root| std::path::Path::new(root) != installation.root)
    {
        bail!("Recorded tool environment disagrees with its installation root");
    }
    let user_home = home()?.canonicalize()?;
    let uid = std::fs::metadata(&user_home)?.uid();
    for path in std::iter::once(&installation.root).chain(installation.environment.iter()) {
        let resolved = path
            .canonicalize()
            .context("Tool installation is no longer present")?;
        if !path.is_absolute()
            || resolved == user_home
            || !resolved.starts_with(&user_home)
            || std::fs::metadata(&resolved)?.uid() != uid
        {
            bail!("Only verified user-owned tool installations inside your home are supported");
        }
    }
    if let Some(environment) = &installation.environment {
        let root = installation.root.canonicalize()?;
        if !environment.canonicalize()?.starts_with(&root)
            || environment.symlink_metadata()?.file_type().is_symlink()
        {
            bail!("Tool environment is not a dedicated directory inside its manager root");
        }
    }
    if !installation.manager.is_absolute() || !installation.manager.is_file() {
        bail!("The recorded tool manager is unavailable");
    }
    for executable in &installation.executables {
        let name = executable
            .file_name()
            .and_then(|s| s.to_str())
            .context("Invalid executable path")?;
        validate_name(name)?;
        if !executable.is_absolute() {
            bail!("Executable path must be absolute");
        }
        let parent = executable
            .parent()
            .context("Executable has no directory")?
            .canonicalize()?;
        if !parent.starts_with(&user_home) || std::fs::metadata(&parent)?.uid() != uid {
            bail!("Tool entry point directory is not user-owned");
        }
        if let Ok(resolved) = executable.canonicalize() {
            if let Some(environment) = &installation.environment {
                if !resolved.starts_with(environment.canonicalize()?) {
                    bail!("Exposed command belongs to another installation");
                }
            } else if executable.symlink_metadata()?.file_type().is_symlink() {
                bail!("Cargo binary ownership cannot be verified through a symbolic link");
            }
        }
        if let Ok(running) = std::env::current_exe().and_then(|p| p.canonicalize()) {
            if executable.canonicalize().ok().as_ref() == Some(&running) {
                bail!("Scope cannot remove or replace its running executable");
            }
        }
        if let Ok(resolved) = executable.canonicalize() {
            let owns_manager = std::iter::once(installation.manager.clone())
                .chain(
                    [
                        "uv", "pipx", "cargo", "rustup", "npm", "bun", "node", "corepack",
                    ]
                    .iter()
                    .filter_map(|name| crate::domain::system::resolve(name)),
                )
                .any(|manager| manager.canonicalize().ok().as_ref() == Some(&resolved));
            if owns_manager {
                bail!(
                    "The installation owns a tool manager and cannot be removed or replaced here"
                );
            }
        }
    }
    if javascript {
        super::javascript_safety::validate(installation)?;
    }
    Ok(())
}
