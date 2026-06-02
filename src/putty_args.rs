use std::fmt;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};

use crate::config::host::{Host, HostSource};

#[derive(Clone, PartialEq, Eq)]
pub struct PuttyLaunch {
    pub host: String,
    pub user: Option<String>,
    pub port: u16,
    pub identity_file: Option<PathBuf>,
    pub temporary_password: Option<String>,
}

impl fmt::Debug for PuttyLaunch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PuttyLaunch")
            .field("host", &self.host)
            .field("user", &self.user)
            .field("port", &self.port)
            .field("identity_file", &self.identity_file)
            .field(
                "temporary_password",
                &self.temporary_password.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl PuttyLaunch {
    pub fn to_transient_host(&self, default_user: &str) -> Result<Host> {
        let user = self
            .user
            .clone()
            .filter(|candidate| !candidate.is_empty())
            .or_else(|| (!default_user.is_empty()).then(|| default_user.to_string()))
            .ok_or_else(|| anyhow!("missing user for PuTTY SSH launch"))?;
        let identity_files = self
            .identity_file
            .clone()
            .map(|path| vec![path])
            .unwrap_or_default();

        Ok(Host {
            id: format!("putty:{}@{}:{}", user, self.host, self.port),
            alias: format!("{}@{}", user, self.host),
            hostname: self.host.clone(),
            port: self.port,
            user,
            identity_files,
            proxy_jump: None,
            tags: vec![],
            description: "PuTTY compatibility one-shot launch".into(),
            source: HostSource::Manual,
            forwards: vec![],
        })
    }
}

pub fn parse_putty_args<I>(args: I) -> Result<PuttyLaunch>
where
    I: IntoIterator<Item = String>,
{
    let mut iter = args.into_iter().peekable();
    let mut explicit_user: Option<String> = None;
    let mut host_token: Option<String> = None;
    let mut port = 22;
    let mut identity_file = None;
    let mut temporary_password = None;

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-ssh" => {}
            "-l" => explicit_user = Some(next_value(&mut iter, "-l")?),
            "-P" => {
                let value = next_value(&mut iter, "-P")?;
                port = value
                    .parse::<u16>()
                    .ok()
                    .filter(|parsed| *parsed > 0)
                    .ok_or_else(|| anyhow!("invalid PuTTY port: {value}"))?;
            }
            "-i" => identity_file = Some(PathBuf::from(next_value(&mut iter, "-i")?)),
            "-pw" => temporary_password = Some(next_value(&mut iter, "-pw")?),
            "-load" => bail!("PuTTY saved sessions are not supported"),
            "-telnet" | "-raw" | "-rlogin" | "-serial" => {
                bail!("unsupported PuTTY connection mode: {arg}")
            }
            _ if arg.starts_with('-') => bail!("unsupported PuTTY option: {arg}"),
            _ => {
                if host_token.replace(arg).is_some() {
                    bail!("multiple hosts supplied for PuTTY SSH launch");
                }
            }
        }
    }

    let raw_host = host_token.ok_or_else(|| anyhow!("missing host for PuTTY SSH launch"))?;
    let (host_user, host) = split_user_host(&raw_host)?;
    let user = explicit_user.or(host_user);

    Ok(PuttyLaunch {
        host,
        user,
        port,
        identity_file,
        temporary_password,
    })
}

fn next_value<I>(iter: &mut std::iter::Peekable<I>, option: &str) -> Result<String>
where
    I: Iterator<Item = String>,
{
    iter.next()
        .ok_or_else(|| anyhow!("missing value for PuTTY option {option}"))
}

fn split_user_host(raw: &str) -> Result<(Option<String>, String)> {
    let (user, host) = raw
        .split_once('@')
        .map(|(user, host)| (Some(user.to_string()), host.to_string()))
        .unwrap_or_else(|| (None, raw.to_string()));

    if matches!(user.as_deref(), Some("")) {
        bail!("missing user before @ in PuTTY host");
    }
    if host.is_empty() {
        bail!("missing host for PuTTY SSH launch");
    }

    Ok((user, host))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse(args: &[&str]) -> anyhow::Result<PuttyLaunch> {
        parse_putty_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn putty_args_parse_login_port_and_host() {
        let launch = parse(&["-ssh", "-l", "deploy", "-P", "2200", "prod.example.com"]).unwrap();

        assert_eq!(launch.host, "prod.example.com");
        assert_eq!(launch.user.as_deref(), Some("deploy"));
        assert_eq!(launch.port, 2200);
    }

    #[test]
    fn putty_args_parse_user_at_host_with_port_after_host() {
        let launch = parse(&["-ssh", "deploy@prod.example.com", "-P", "2200"]).unwrap();

        assert_eq!(launch.host, "prod.example.com");
        assert_eq!(launch.user.as_deref(), Some("deploy"));
        assert_eq!(launch.port, 2200);
    }

    #[test]
    fn putty_args_login_flag_wins_over_user_at_host() {
        let launch = parse(&["-ssh", "-l", "admin", "deploy@prod.example.com"]).unwrap();

        assert_eq!(launch.host, "prod.example.com");
        assert_eq!(launch.user.as_deref(), Some("admin"));
    }

    #[test]
    fn putty_args_default_port_is_22() {
        let launch = parse(&["-ssh", "-l", "deploy", "prod.example.com"]).unwrap();

        assert_eq!(launch.port, 22);
    }

    #[test]
    fn putty_args_parse_identity_file_and_temporary_password() {
        let launch = parse(&[
            "-ssh",
            "-l",
            "deploy",
            "-i",
            "~/.ssh/id_ed25519",
            "-pw",
            "secret",
            "prod.example.com",
        ])
        .unwrap();

        assert_eq!(launch.identity_file, Some(PathBuf::from("~/.ssh/id_ed25519")));
        assert_eq!(launch.temporary_password.as_deref(), Some("secret"));
    }

    #[test]
    fn putty_args_debug_does_not_include_temporary_password() {
        let launch = parse(&["-ssh", "-l", "deploy", "-pw", "secret", "prod.example.com"]).unwrap();

        assert!(!format!("{launch:?}").contains("secret"));
    }

    #[test]
    fn putty_args_reject_missing_host() {
        let error = parse(&["-ssh", "-l", "deploy"]).unwrap_err().to_string();

        assert!(error.contains("missing host"));
    }

    #[test]
    fn putty_args_reject_bad_port() {
        let error = parse(&["-ssh", "-P", "bad", "prod.example.com"])
            .unwrap_err()
            .to_string();

        assert!(error.contains("invalid PuTTY port"));
    }

    #[test]
    fn putty_args_reject_saved_session_load() {
        let error = parse(&["-load", "prod"]).unwrap_err().to_string();

        assert!(error.contains("saved sessions are not supported"));
    }

    #[test]
    fn putty_args_reject_unsupported_option_instead_of_host() {
        let error = parse(&["-ssh", "-L", "8080:localhost:80", "prod.example.com"])
            .unwrap_err()
            .to_string();

        assert!(error.contains("unsupported PuTTY option: -L"));
    }

    #[test]
    fn putty_args_reject_multiple_hosts() {
        let error = parse(&["-ssh", "one.example.com", "two.example.com"])
            .unwrap_err()
            .to_string();

        assert!(error.contains("multiple hosts"));
    }

    #[test]
    fn putty_launch_builds_transient_host_with_default_user() {
        let launch = parse(&["-ssh", "-i", "~/.ssh/id_ed25519", "prod.example.com"]).unwrap();
        let host = launch.to_transient_host("deploy").unwrap();

        assert_eq!(host.id, "putty:deploy@prod.example.com:22");
        assert_eq!(host.alias, "deploy@prod.example.com");
        assert_eq!(host.hostname, "prod.example.com");
        assert_eq!(host.user, "deploy");
        assert_eq!(host.identity_files, vec![PathBuf::from("~/.ssh/id_ed25519")]);
    }
}
