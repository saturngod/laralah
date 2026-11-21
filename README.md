<p align="center">
  <img src="icon.png" alt="Laralah icon" width="128" height="128">
</p>

<h1 align="center">Laralah</h1>

<p align="center">
  A fast desktop app for running PHP code inside your Laravel project.
</p>

---

## What is Laralah?

Laralah is a scratchpad for Laravel. You write PHP code, press **Cmd+R**, and see the result right away. It runs your code inside your real Laravel app, so you can use your models, helpers, and database.

It is a native app written in Rust (with [GPUI](https://www.gpui.rs/)). It runs on macOS, Linux, and Windows.

## Features

- **Run code fast** – write PHP and run it in your Laravel project.
- **Playground** (Cmd+Shift+O) – no project? Laralah sets up a fresh Laravel app (latest stable, SQLite database) in `~/.laralah/playground` so you can try code right away. Needs [Composer](https://getcomposer.org) the first time. Use **Reset Playground Database** in the command palette for a clean database.
- **Three ways to see results**
  - **Detail** (Cmd+1) – see each value step by step.
  - **Table** (Cmd+2) – see lists and collections as a table.
  - **Raw CLI** (Cmd+3) – see the plain output.
- **SQL queries** (Cmd+S) – see every query your code runs. Duplicate queries are marked.
- **Object graph** – see how your objects connect. Zoom with `+`, `-`, and `0`.
- **Logs** (Cmd+L) – read and search your Laravel log files.
- **Mail preview** – look at your mailables.
- **Tabs** – keep many scratch tabs open at the same time.
- **SSH and Docker** – run code on a remote server, in a Docker container on that server, or in a Docker container on your own computer (Connection Type: Local Docker, no SSH needed).
- **AI helper** (Cmd+I) – ask an AI coding tool for help. Works with Claude Code, Codex, OpenCode, Qoder, and Antigravity, if you have them installed.
- **Command palette** (Cmd+P) – find any action quickly.
- **Syntax check** – PHP errors show up before you run.
- **Format code** (Cmd+Shift+F) – tidy up your code with [Laravel Pint](https://laravel.com/docs/pint), using your project's `pint.json`. Without Pint, Laralah fixes the indentation for you.

## Requirements

- PHP on your computer (or on the server / container you connect to)
- A Laravel project, or [Composer](https://getcomposer.org) to use the Playground

## Install

Download the latest version from the [Releases page](https://github.com/saturngod/laralah/releases).

| System  | File |
|---------|------|
| macOS   | `.dmg` (Apple Silicon or Intel) |
| Linux   | `.deb` or `.tar.gz` (x86_64 or aarch64) |
| Windows | `.zip` (x64 or ARM64) |

## How to use

1. Open Laralah.
2. Press **Cmd+O** and pick your Laravel project folder.
3. Write some PHP, for example:
   ```php
   User::where('active', true)->take(5)->get();
   ```
4. Press **Cmd+R** to run it.

On Linux and Windows, use **Ctrl** where this page says **Cmd**. Every command is also in the menu: the menu bar on macOS, or the **☰** button at the top left on Linux and Windows (File, Edit, Editor, View).

## Keyboard shortcuts

| Shortcut | Action |
|----------|--------|
| Cmd+R | Run code |
| Cmd+Shift+F | Format code |
| Cmd+O | Open a project folder |
| Cmd+Shift+O | Open the Playground |
| Cmd+T | New tab |
| Cmd+W | Close tab |
| Cmd+Shift+] / Cmd+Shift+[ | Next / previous tab |
| Cmd+P | Command palette |
| Cmd+K | SSH connections |
| Cmd+L | Show logs |
| Cmd+S | Show SQL queries |
| Cmd+I | AI sidebar |
| Cmd+B | Show / hide sidebar |
| Cmd+1 / 2 / 3 | Detail / Table / Raw CLI view |
| Cmd+, | Settings |
| Esc | Close the open panel |

On Linux and Windows, Ctrl+Tab / Ctrl+Shift+Tab also switch tabs, and Alt+/ opens autocomplete when your input method uses Ctrl+Space. You can change any shortcut in Settings → Shortcuts.

## Build from source

You need [Rust](https://rustup.rs/) (stable).

```bash
git clone https://github.com/saturngod/laralah.git
cd laralah
cargo run --release
```

### Linux packages

Install these first (Ubuntu / Debian):

```bash
sudo apt-get install -y pkg-config clang cmake libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libx11-dev libx11-xcb-dev libxcb1-dev libxi-dev libxrandr-dev libfontconfig-dev libfreetype-dev libasound2-dev libssl-dev libzstd-dev
```

### Make a package

| System  | Command | Output |
|---------|---------|--------|
| macOS   | `scripts/package-macos.sh` | `dist/*.dmg` |
| Linux   | `scripts/package-linux.sh` | `dist/*.deb`, `dist/*.tar.gz` |
| Windows | `pwsh scripts/package-windows.ps1` | `dist/*.zip` |

To install on Linux for your user only (into `~/.local`):

```bash
scripts/install-linux.sh
```

### Run tests

```bash
cargo test
```

## Release

Push a version tag. GitHub Actions builds every platform and makes a draft release. The tag must match the version in `Cargo.toml`.

```bash
git tag v0.1.0
git push origin v0.1.0
```
