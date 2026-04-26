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
- Path-type tags build a virtual folder sidebar right inside the main view
- Embedded terminal emulator: `vim`, `tmux`, `htop` all work correctly

**Seamless SSH ↔ SFTP switching**
- `Ctrl-\` flips between SSH shell and SFTP browser
- SSH and SFTP share a single TCP connection — no re-authentication
- Your working directory context is preserved

**SFTP that doesn't suck**
- Wide terminals show local and remote panels side by side; narrow terminals show only the active pane
- `Tab` to switch focus between local and remote panes without losing each side's selection
- `d` to download, `u` to upload, with a global bottom-right transfer badge
- Directory transfers keep the selected directory itself and show aggregate `N/M` progress
- `e` to open a remote file in your system's default GUI app and auto-upload on save
- `Enter` to navigate directories
- One connection-scoped FIFO queue keeps transfers running while you move between Main, SSH, and SFTP

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
| `f` | Toggle folder sidebar |
| `j` | Jump to folder (when folders are focused) |
| `q` | Quit |

When the folder sidebar is visible, search is scoped to the current folder and the search box shows a read-only `path:/current/folder` prefix.

**SSH mode**
| Key | Action |
|-----|--------|
| `Ctrl-\` | Switch to SFTP browser |
| `exit` / `Ctrl-D` | Disconnect, return to host list |

**SFTP mode**
| Key | Action |
|-----|--------|
| `Tab` | Switch focus between local / remote panes |
| `Space` | Toggle selection on the focused row |
| `Space` × 2 | Select the inclusive range from the anchor to the focused row |
| `Esc` | Cancel multi-select for the active pane |
| `Enter` | Open directory |
| `d` | Download the selected remote item, or all selected remote items in multi-select mode |
| `u` | Upload the selected local item, or all selected local items in multi-select mode |
| `D` | Delete all selected items in the active pane |
| `e` | Edit selected remote file locally |
| `Ctrl-\` | Switch back to SSH shell |
| `Ctrl-C` × 2 | Return to host list |

When you press `e` on a remote file, `sush` downloads it into a temporary workspace, opens it with the operating system's default app, watches for changes, and auto-uploads after each save. Auto-upload writes to a temporary remote file first, moves the old target aside when needed, and then switches the new file into place.

When you transfer a directory, `sush` preserves the selected directory itself at the destination, prepares nested directories first, and then transfers files one by one while the queue badge shows `current/total` plus the current file percentage for the active file.

In SFTP multi-select mode, each pane keeps its own selection set. Press `Space` once to toggle the focused row and set the anchor. Press `Space` twice quickly on another row to select the inclusive range between the anchor and the current row. While multi-select is active, the status bar switches to batch-only actions: local pane shows `u / D / Esc`, remote pane shows `d / D / Esc`.

Transfers now run through a single FIFO queue scoped to the current SSH connection. The bottom-right corner of Main, SSH, and SFTP shows a compact badge like `↑ 2/10 37%` or `↓ 2/10 37%`, so long-running transfers continue in the background without taking over the entire status line. Disconnecting the current connection clears the queue.

For normal files, repeated uploads and downloads now resume from the existing target size when it is smaller than or equal to the source. If the target is larger than the source, `sush` restarts that file from zero. This first version does not add hash verification or cross-restart resume records.

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
| **v0.4** ✅ | Connection history · recency-boosted search · TCP connectivity probe |
| **v0.5** ✅ | Path-type tags · main-view folder sidebar · folder jump · scoped `path:` search |
| **v0.6** ✅ | System keyring credential storage · silent save after successful auth · temporary input only when Secret Service is unavailable |
| **v0.7** ✅ | Recursive folder transfer with aggregate progress · remote file editing with auto-upload on save · dual-pane SFTP · background transfer queue · resume support |
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
