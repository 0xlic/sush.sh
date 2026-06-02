use std::fs;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::store::PuttyCompatMetadata;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PuttyShimStatus {
    pub supported: bool,
    pub enabled: bool,
    pub shim_path: Option<PathBuf>,
    pub message: String,
    pub next_step: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShimConfig {
    pub sush_exe_path: PathBuf,
}

pub fn current_platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOs
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::Other
    }
}

pub fn managed_shim_path(config_dir: &Path) -> PathBuf {
    config_dir.join("putty-compat").join("putty.exe")
}

pub fn shim_config_path(config_dir: &Path) -> PathBuf {
    config_dir.join("putty-compat").join("putty-shim.toml")
}

pub fn status(metadata: &PuttyCompatMetadata, config_dir: &Path) -> PuttyShimStatus {
    status_for_platform(metadata, config_dir, current_platform())
}

pub fn status_for_platform(
    metadata: &PuttyCompatMetadata,
    config_dir: &Path,
    platform: Platform,
) -> PuttyShimStatus {
    let supported = platform == Platform::Windows;
    let expected_path = managed_shim_path(config_dir);
    if !supported {
        return PuttyShimStatus {
            supported: false,
            enabled: false,
            shim_path: Some(expected_path),
            message: "PuTTY shim automatic installation is not supported on this platform.".into(),
            next_step: "Use Windows Settings to install the managed putty.exe shim.".into(),
        };
    }

    let enabled = metadata.enabled;
    let shim_path = metadata
        .shim_path
        .clone()
        .or_else(|| Some(expected_path.clone()));
    let message = if enabled {
        "PuTTY compatibility launcher is enabled.".into()
    } else {
        "PuTTY compatibility launcher is disabled.".into()
    };
    let next_step = if enabled {
        format!(
            "Configure your bastion client PuTTY path to {}.",
            shim_path.as_ref().unwrap_or(&expected_path).display()
        )
    } else {
        "Press Space to install the sush-managed putty.exe shim.".into()
    };

    PuttyShimStatus {
        supported,
        enabled,
        shim_path,
        message,
        next_step,
    }
}

pub fn enable(config_dir: &Path, metadata: &mut PuttyCompatMetadata) -> Result<PuttyShimStatus> {
    if current_platform() != Platform::Windows {
        metadata.enabled = false;
        metadata.last_error =
            Some("PuTTY shim automatic installation is only supported on Windows.".into());
        return Ok(status(metadata, config_dir));
    }

    let shim_path = managed_shim_path(config_dir);
    let sidecar_path = shim_config_path(config_dir);
    if shim_path.exists() && !sidecar_path.exists() {
        bail!(
            "refusing to overwrite unmanaged PuTTY shim at {}",
            shim_path.display()
        );
    }

    let sush_exe_path =
        std::env::current_exe().context("failed to locate current sush executable")?;
    if let Some(parent) = shim_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(&sush_exe_path, &shim_path).with_context(|| {
        format!(
            "failed to install PuTTY shim from {} to {}",
            sush_exe_path.display(),
            shim_path.display()
        )
    })?;
    fs::write(
        &sidecar_path,
        encode_shim_config(&ShimConfig {
            sush_exe_path: sush_exe_path.clone(),
        })?,
    )
    .with_context(|| format!("failed to write {}", sidecar_path.display()))?;

    metadata.enabled = true;
    metadata.shim_path = Some(shim_path);
    metadata.sush_exe_path = Some(sush_exe_path);
    metadata.last_error = None;
    Ok(status(metadata, config_dir))
}

pub fn disable(config_dir: &Path, metadata: &mut PuttyCompatMetadata) -> Result<PuttyShimStatus> {
    if current_platform() == Platform::Windows {
        let shim_path = metadata
            .shim_path
            .clone()
            .unwrap_or_else(|| managed_shim_path(config_dir));
        let sidecar_path = shim_config_path(config_dir);
        if shim_path.exists() && sidecar_path.exists() {
            fs::remove_file(&shim_path)
                .with_context(|| format!("failed to remove {}", shim_path.display()))?;
        }
        if sidecar_path.exists() {
            fs::remove_file(&sidecar_path)
                .with_context(|| format!("failed to remove {}", sidecar_path.display()))?;
        }
    }

    metadata.enabled = false;
    metadata.shim_path = None;
    metadata.sush_exe_path = None;
    metadata.last_error = None;
    Ok(status(metadata, config_dir))
}

pub fn encode_shim_config(config: &ShimConfig) -> Result<String> {
    toml::to_string(config).context("failed to encode PuTTY shim config")
}

pub fn decode_shim_config(content: &str) -> Result<ShimConfig> {
    toml::from_str(content).context("failed to decode PuTTY shim config")
}

pub fn is_putty_shim_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("putty.exe") || name == "putty")
}

pub fn is_current_putty_shim() -> bool {
    std::env::current_exe()
        .ok()
        .as_deref()
        .is_some_and(is_putty_shim_path)
}

pub fn launch_terminal_from_current_shim(args: &[String]) -> Result<()> {
    let shim_exe = std::env::current_exe().context("failed to locate PuTTY shim executable")?;
    let config_path = shim_exe
        .parent()
        .map(|parent| parent.join("putty-shim.toml"))
        .ok_or_else(|| anyhow::anyhow!("failed to locate PuTTY shim config directory"))?;
    let config = decode_shim_config(
        &fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?,
    )?;
    launch_terminal(&config.sush_exe_path, args)
}

pub fn launch_terminal(sush_exe: &Path, args: &[String]) -> Result<()> {
    #[cfg(windows)]
    {
        let candidates = terminal_launch_candidates(sush_exe, args);
        let mut last_error = None;
        for candidate in candidates {
            let Some((program, rest)) = candidate.split_first() else {
                continue;
            };
            match Command::new(program).args(rest).spawn() {
                Ok(_) => return Ok(()),
                Err(error) => last_error = Some(error),
            }
        }
        bail!(
            "failed to launch terminal for PuTTY compatibility: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "no launch command was available".into())
        );
    }

    #[cfg(not(windows))]
    {
        let _ = (sush_exe, args);
        bail!("PuTTY shim terminal launch is only supported on Windows");
    }
}

#[cfg(any(test, windows))]
pub fn terminal_launch_candidates(sush_exe: &Path, args: &[String]) -> Vec<Vec<String>> {
    let compat_args = std::iter::once("--putty-compatible".to_string())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>();
    let sush = sush_exe.display().to_string();

    vec![
        std::iter::once("wt.exe".to_string())
            .chain([
                "new-tab".to_string(),
                "--title".to_string(),
                "sush".to_string(),
            ])
            .chain(std::iter::once(sush.clone()))
            .chain(compat_args.clone())
            .collect(),
        std::iter::once("cmd.exe".to_string())
            .chain([
                "/C".to_string(),
                "start".to_string(),
                "sush".to_string(),
                sush,
            ])
            .chain(compat_args)
            .collect(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn putty_shim_path_is_sush_managed() {
        let config_dir = std::path::Path::new("/home/me/.config/sush");

        assert_eq!(
            managed_shim_path(config_dir),
            config_dir.join("putty-compat").join("putty.exe")
        );
    }

    #[test]
    fn putty_shim_sidecar_path_is_next_to_shim() {
        let config_dir = std::path::Path::new("/home/me/.config/sush");

        assert_eq!(
            shim_config_path(config_dir),
            config_dir.join("putty-compat").join("putty-shim.toml")
        );
    }

    #[test]
    fn putty_shim_status_reports_unsupported_on_non_windows() {
        let metadata = crate::config::store::PuttyCompatMetadata::default();
        let status = status_for_platform(
            &metadata,
            std::path::Path::new("/home/me/.config/sush"),
            Platform::MacOs,
        );

        assert!(!status.supported);
        assert!(!status.enabled);
        assert!(status.message.contains("not supported"));
    }

    #[test]
    fn putty_shim_status_reports_linux_and_other_as_unsupported() {
        let metadata = crate::config::store::PuttyCompatMetadata::default();
        let config_dir = std::path::Path::new("/home/me/.config/sush");

        assert!(!status_for_platform(&metadata, config_dir, Platform::Linux).supported);
        assert!(!status_for_platform(&metadata, config_dir, Platform::Other).supported);
    }

    #[test]
    fn putty_shim_status_reports_windows_next_step() {
        let metadata = crate::config::store::PuttyCompatMetadata {
            enabled: true,
            shim_path: Some(std::path::PathBuf::from(
                "C:/Users/me/.config/sush/putty-compat/putty.exe",
            )),
            sush_exe_path: Some(std::path::PathBuf::from("C:/Tools/sush.exe")),
            last_error: None,
        };
        let status = status_for_platform(
            &metadata,
            std::path::Path::new("C:/Users/me/.config/sush"),
            Platform::Windows,
        );

        assert!(status.supported);
        assert!(status.enabled);
        assert!(status.next_step.contains("putty.exe"));
    }

    #[test]
    fn putty_shim_config_roundtrips_real_sush_path() {
        let config = ShimConfig {
            sush_exe_path: std::path::PathBuf::from("C:/Tools/sush.exe"),
        };
        let encoded = encode_shim_config(&config).unwrap();
        let decoded = decode_shim_config(&encoded).unwrap();

        assert_eq!(
            decoded.sush_exe_path,
            std::path::PathBuf::from("C:/Tools/sush.exe")
        );
    }

    #[test]
    fn putty_shim_current_status_is_available() {
        let metadata = crate::config::store::PuttyCompatMetadata::default();
        let dir = tempfile::TempDir::new().unwrap();

        let current_status = status(&metadata, dir.path());

        assert_eq!(
            current_status.supported,
            current_platform() == Platform::Windows
        );
    }

    #[test]
    fn putty_shim_enable_disable_updates_metadata_for_current_platform() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut metadata = crate::config::store::PuttyCompatMetadata::default();

        let enabled_status = enable(dir.path(), &mut metadata).unwrap();

        if current_platform() == Platform::Windows {
            assert!(enabled_status.enabled);
            assert!(
                metadata
                    .shim_path
                    .as_ref()
                    .is_some_and(|path| path.exists())
            );
        } else {
            assert!(!enabled_status.enabled);
            assert!(metadata.last_error.is_some());
        }

        let disabled_status = disable(dir.path(), &mut metadata).unwrap();
        assert!(!disabled_status.enabled);
        assert!(!metadata.enabled);
    }

    #[test]
    fn putty_direct_shim_detection_matches_putty_exe_name() {
        assert!(is_putty_shim_path(std::path::Path::new(
            "C:/Tools/putty.exe"
        )));
        assert!(is_putty_shim_path(std::path::Path::new("putty")));
        assert!(!is_putty_shim_path(std::path::Path::new("sush.exe")));
    }

    #[test]
    fn putty_direct_terminal_command_adds_compat_marker() {
        let args = vec!["-ssh".to_string(), "deploy@prod.example.com".to_string()];
        let candidates =
            terminal_launch_candidates(std::path::Path::new("C:/Tools/sush.exe"), &args);

        assert!(candidates.iter().any(|candidate| {
            candidate.iter().any(|part| part == "--putty-compatible")
                && candidate
                    .iter()
                    .any(|part| part == "deploy@prod.example.com")
        }));
    }
}
