use anyhow::{Context, Result};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileType;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::ssh::session::ActiveSession;

pub struct SftpClient {
    pub session: SftpSession,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

impl SftpClient {
    pub async fn open(ssh: &ActiveSession) -> Result<Self> {
        let channel = ssh.handle.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .context("failed to open SFTP subsystem")?;
        Ok(Self { session: sftp })
    }

    pub async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>> {
        let entries = self
            .session
            .read_dir(path)
            .await
            .with_context(|| format!("failed to list directory: {path}"))?;
        let mut result: Vec<FileEntry> = entries
            .map(|e| FileEntry {
                name: e.file_name(),
                is_dir: matches!(e.file_type(), FileType::Dir),
                size: e.metadata().size.unwrap_or(0),
            })
            .collect();
        // Directories first, files after, each sorted by name ascending.
        result.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });
        Ok(result)
    }

    pub async fn home_dir(&self) -> String {
        self.session
            .canonicalize(".")
            .await
            .unwrap_or_else(|_| "/".into())
    }

    pub async fn download_file_to_path(&self, remote_path: &str, local_path: &Path) -> Result<()> {
        let mut remote = self
            .session
            .open(remote_path)
            .await
            .with_context(|| format!("failed to open remote file: {remote_path}"))?;
        let mut local = tokio::fs::File::create(local_path)
            .await
            .with_context(|| format!("failed to create local file: {}", local_path.display()))?;
        let mut buffer = vec![0u8; 32 * 1024];

        loop {
            let count = remote.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            local.write_all(&buffer[..count]).await?;
        }

        Ok(())
    }

    pub async fn upload_file_from_path(&self, local_path: &Path, remote_path: &str) -> Result<()> {
        let temp_remote_path = build_remote_edit_temp_path(remote_path);
        let mut local = tokio::fs::File::open(local_path)
            .await
            .with_context(|| format!("failed to open local file: {}", local_path.display()))?;
        let mut remote = self
            .session
            .create(&temp_remote_path)
            .await
            .with_context(|| format!("failed to create remote file: {temp_remote_path}"))?;
        let mut buffer = vec![0u8; 32 * 1024];

        loop {
            let count = local.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            remote.write_all(&buffer[..count]).await?;
        }

        drop(remote);
        self.session
            .rename(&temp_remote_path, remote_path)
            .await
            .with_context(|| {
                format!("failed to replace remote file: {temp_remote_path} -> {remote_path}")
            })?;

        Ok(())
    }
}

fn build_remote_child_path(parent: &str, name: &str) -> String {
    match parent {
        "/" => format!("/{name}"),
        _ if parent.ends_with('/') => format!("{parent}{name}"),
        _ => format!("{parent}/{name}"),
    }
}

fn build_remote_edit_temp_path(remote_path: &str) -> String {
    let path = Path::new(remote_path);
    let parent = path
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".into());
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "remote-edit".into());
    build_remote_child_path(&parent, &format!(".{filename}.sush-upload.tmp"))
}

pub fn list_local(path: &std::path::Path) -> Result<Vec<FileEntry>> {
    let mut entries: Vec<FileEntry> = std::fs::read_dir(path)?
        .filter_map(|e| e.ok())
        .map(|e| {
            let meta = e.metadata().ok();
            FileEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir: meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            }
        })
        .collect();
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_remote_child_path_joins_under_directory() {
        assert_eq!(build_remote_child_path("/etc", "hosts"), "/etc/hosts");
    }

    #[test]
    fn build_remote_child_path_avoids_double_slash_for_root() {
        assert_eq!(build_remote_child_path("/", "hosts"), "/hosts");
    }

    #[test]
    fn build_remote_edit_temp_path_stays_in_same_directory() {
        assert_eq!(
            build_remote_edit_temp_path("/etc/hosts"),
            "/etc/.hosts.sush-upload.tmp"
        );
    }

    #[test]
    fn remote_edit_transfer_helpers_exist() {
        let _download = SftpClient::download_file_to_path;
        let _upload = SftpClient::upload_file_from_path;
    }
}
