use anyhow::{Context, Result};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileType;

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
            .context("打开 SFTP subsystem 失败")?;
        Ok(Self { session: sftp })
    }

    pub async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>> {
        let entries = self
            .session
            .read_dir(path)
            .await
            .with_context(|| format!("列目录失败: {path}"))?;
        let mut result: Vec<FileEntry> = entries
            .map(|e| FileEntry {
                name: e.file_name(),
                is_dir: matches!(e.file_type(), FileType::Dir),
                size: e.metadata().size.unwrap_or(0),
            })
            .collect();
        // 目录在前、文件在后，各自按名称升序
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
