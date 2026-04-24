# sush

> SSH and SFTP, finally living under the same roof.

`sush` is a tiny, fast, terminal-native tool for managing SSH connections and SFTP file transfers — without ever leaving your keyboard.

**[中文文档 →](docs/README.zh.md)**

---

## The problem

You SSH into a server. Then you realize you need to grab a file. So you:

1. Open a new terminal tab
2. Fumble with `sftp user@host`
3. Forget the path you were just looking at
4. Give up and use `scp` from memory
5. Get the path wrong anyway

`sush` fixes this by treating SSH and SFTP as two views of the same session — press `Ctrl-\` to flip between them. That's it.

---

## Demo

```
┌─ sush ─────────────────────────────────────────────┐
│                                                     │
│  > prod█                                            │
│                                                     │
│  ┌───────────────────────────────────────────────┐  │
│  │ ● prod-web-01   192.168.1.10          web    │  │
│  │   prod-db-01    192.168.1.20           db    │  │
│  │   prod-cache    192.168.1.30        cache    │  │
│  └───────────────────────────────────────────────┘  │
│                                                     │
│  /:search  Enter:SSH  s:SFTP  q:quit               │
└─────────────────────────────────────────────────────┘
```

Type to fuzzy-search. Hit Enter to connect. Hit `Ctrl-\` at any time to switch to the SFTP browser (your SSH session stays alive). Hit `Ctrl-\` again to jump back.

---

## Features

**Zero friction SSH**
- Reads `~/.ssh/config` automatically on startup — your hosts are already there
- Fuzzy search across hostname, IP, user, tags, and description
- Embedded terminal emulator: `vim`, `tmux`, `htop` all work correctly

**Seamless SSH ↔ SFTP switching**
- `Ctrl-\` flips between SSH shell and SFTP browser
- SSH and SFTP share a single TCP connection — no re-authentication
- Your working directory context is preserved

**SFTP that doesn't suck**
- `Tab` to switch between local and remote panels
- `d` to download, `u` to upload, with a live progress bar
- `Enter` to navigate directories

**Snappy**
- Starts in under 200ms
- Search responds in under 50ms
- Idle memory under 30MB

---

## Install

### From binary (recommended)

Download the latest release for your platform from [GitHub Releases](https://github.com/lichen/sush.sh/releases):

| Platform       | File                        |
|----------------|-----------------------------|
| macOS (Apple)  | `sush-macos-arm64`          |
| macOS (Intel)  | `sush-macos-x86_64`         |
| Linux x86_64   | `sush-linux-x86_64`         |
| Windows x86_64 | `sush-windows-x86_64.exe`   |

```sh
# macOS / Linux
chmod +x sush-*
mv sush-* /usr/local/bin/sush
sush
```

### From source

```sh
git clone https://github.com/lichen/sush.sh
cd sush.sh
cargo build --release
./target/release/sush
```

Requires Rust 1.95+. No other dependencies.

---

## Quickstart

```sh
sush
```

On first launch, `sush` will ask whether to import from `~/.ssh/config`. You can also press `n` to add hosts manually, or `i` to import at any time.

**Navigation**
| Key | Action |
|-----|--------|
| `/` or just type | Focus search |
| `↑` / `↓` | Move through host list |
| `Enter` | Connect via SSH |
| `s` | Open SFTP browser |
| `n` | New host |
| `e` | Edit selected host |
| `d` | Delete selected host |
| `i` | Import from `~/.ssh/config` |
| `q` | Quit |

**SSH mode**
| Key | Action |
|-----|--------|
| `Ctrl-\` | Switch to SFTP browser |
| `exit` / `Ctrl-D` | Disconnect, return to host list |

**SFTP mode**
| Key | Action |
|-----|--------|
| `Tab` | Toggle local / remote view |
| `Enter` | Open directory |
| `d` | Download selected file |
| `u` | Upload selected file |
| `Ctrl-\` | Switch back to SSH shell |
| `Ctrl-C` × 2 | Return to host list |

---

## Authentication

`sush` tries auth methods in order:

1. **ssh-agent** — if `SSH_AUTH_SOCK` is set, uses it
2. **IdentityFile** — reads key paths from your `~/.ssh/config`; prompts for passphrase if needed
3. **Password** — shows a password prompt in the TUI if all else fails

---

## How it works

`sush` uses an **embedded terminal emulator** (powered by [alacritty_terminal](https://github.com/alacritty/alacritty)). When you connect to a host, `sush` feeds remote PTY output into an in-process VT100/xterm state machine and renders the result as a ratatui widget — so the sush UI (status bar, key hints) stays visible throughout the session.

- Terminal programs (`vim`, `tmux`, `htop`) work correctly via full VT100 emulation
- `Ctrl-\` is intercepted as a prefix key within the TUI; everything else is forwarded to the remote
- SSH and SFTP share the same TCP connection via separate channels — switching is instant and doesn't re-authenticate

---

## Roadmap

| Version | Focus |
|---------|-------|
| **v0.1** ✅ | SSH connect · SFTP browser · upload/download · `Ctrl-\` switching |
| **v0.2** ✅ | Embedded terminal emulator · TUI visible during SSH sessions |
| **v0.3** ✅ | TUI host editor · tag chip editor · manual SSH config import |
| v0.4 | Connection history · recent-use sorting · connectivity check |
| v0.5 | Path-type tags · virtual folder navigation |
| v0.6 | Credential encryption (master password) |
| v0.7 | Recursive folder transfer · remote file editing · dual-pane SFTP |
| v0.8 | Port forwarding · ProxyJump chains · SOCKS5 proxy |
| v1.0 | Homebrew/AUR/Scoop · man page · full platform testing |

---

## Built with

- [russh](https://github.com/Eugeny/russh) — pure-Rust SSH implementation
- [ratatui](https://ratatui.rs) — terminal UI framework
- [nucleo](https://github.com/helix-editor/nucleo) — fuzzy matcher (same one Helix uses)
- [tokio](https://tokio.rs) — async runtime

Single binary. No system dependencies. No libssh2. No OpenSSL.

---

## License

MIT
