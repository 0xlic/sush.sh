# Changelog

## [1.0.0] - 2026-05-02

### Features

- Fuzzy search across hostname, IP, user, tags, and description with recency-boosted ranking
- Embedded terminal emulator (VT100/xterm via alacritty_terminal) — vim, tmux, htop all work correctly
- Seamless SSH ↔ SFTP switching with `Ctrl-\`; SSH and SFTP share a single TCP connection
- TCP connectivity probe with live status indicator on the host list
- Connection history with automatic recency weighting in search results
- TUI host editor: create, edit, and delete hosts without leaving the terminal
- Tag system: flat tags plus path-type tags that build a virtual folder sidebar
- Folder sidebar with scoped search and jump-to-folder overlay
- SSH config import (`~/.ssh/config`) with manual trigger
- System keyring integration: passwords and key passphrases stored in macOS Keychain, Windows Credential Manager, or Linux Secret Service
- Dual-pane SFTP layout on wide terminals; single-pane on narrow
- Multi-select with range selection; batch upload, download, and delete
- Background transfer queue (FIFO, scoped to the current connection) with a compact status badge
- Recursive directory transfer with aggregate progress display
- Resume support for interrupted transfers
- Remote file editing: open in default GUI app, auto-upload on save
- Port forwarding manager (`p`): local, remote, and dynamic (SOCKS5) rules stored per host
- Single-hop ProxyJump support for SSH, SFTP, and port forwarding
- Background daemon holds forwarding connections after TUI exits; auto-reconnect with backoff
- Six-platform binary releases via GitHub Actions (macOS arm64/x86-64, Linux arm64/x86-64, Windows x86/x86-64)

### Bug Fixes

- SSH connection times out after 10s instead of hanging indefinitely
- Forwarding view preserves host and rule selection across state rebuilds
