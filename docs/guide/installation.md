---
description: Install a verified agent-session-facts binary or build it from crates.io.
---

# Installation

## Install a verified prebuilt binary

Install the latest release without Rust or Cargo:

```bash
curl --proto '=https' --tlsv1.2 -fsSL \
  'https://github.com/wangyuxinwhy/agent-session-facts/releases/latest/download/install.sh' | sh
```

Verify the installed command:

```bash
agent-session-facts --version
agent-session-facts --help
```

The default destination is `~/.local/bin/agent-session-facts`. The installer never uses `sudo`; add `~/.local/bin` to `PATH` if needed.

The installer detects these platforms:

| Platform | Rust target |
|---|---|
| Linux x86_64 | `x86_64-unknown-linux-gnu` |
| Linux arm64 | `aarch64-unknown-linux-gnu` |
| macOS Intel | `x86_64-apple-darwin` |
| macOS Apple Silicon | `aarch64-apple-darwin` |

It downloads the matching archive and sibling SHA-256 file, rejects unexpected archive paths, verifies the binary's reported version, and atomically replaces the destination.

## Install one explicit version

Download the installer before passing options:

```bash
curl --proto '=https' --tlsv1.2 -fsSL \
  'https://github.com/wangyuxinwhy/agent-session-facts/releases/latest/download/install.sh' \
  -o /tmp/agent-session-facts-install.sh
sh /tmp/agent-session-facts-install.sh \
  --version 0.1.1 \
  --bin-dir "$PWD/bin"
```

Supported options:

```text
--version VERSION   install an explicit version instead of latest
--bin-dir DIR       install somewhere other than ~/.local/bin
--base-url URL      override the GitHub Releases root
--dry-run           resolve the target without downloading or changing files
```

The destination must be absolute. Downloads require HTTPS; plain HTTP is accepted only with explicit `--allow-http` for the repository's local end-to-end test.

## Install from crates.io

With Rust 1.95 or newer:

```bash
cargo install agent-session-facts --version 0.1.1 --locked
agent-session-facts --version
```

This compiles locally and can take longer than downloading a native release.

## From source

```bash
git clone https://github.com/wangyuxinwhy/agent-session-facts.git
cd agent-session-facts
cargo install --path . --locked
```

Repository maintainers can verify the complete package, download, checksum, and installation path with `./scripts/test-installer.sh`.
