---
description: Install agent-session-facts from crates.io, a GitHub Release archive, or source.
---

# Installation

## From crates.io

Install the published Rust binary with its locked dependency graph:

```bash
cargo install agent-session-facts --locked
```

Verify the installed command:

```bash
agent-session-facts --version
```

The minimum supported Rust version for building from source is 1.95.

## From a GitHub Release

Each `vX.Y.Z` GitHub Release contains archives for:

| Platform | Rust target |
|---|---|
| Linux x86_64 | `x86_64-unknown-linux-gnu` |
| Linux arm64 | `aarch64-unknown-linux-gnu` |
| macOS Intel | `x86_64-apple-darwin` |
| macOS Apple Silicon | `aarch64-apple-darwin` |

Download the archive for your platform from the [latest release](https://github.com/wangyuxinwhy/agent-session-facts/releases/latest), verify it with the adjacent `.sha256` file, extract it, and place `agent-session-facts` on your `PATH`.

## From source

```bash
git clone https://github.com/wangyuxinwhy/agent-session-facts.git
cd agent-session-facts
cargo install --path . --locked
```
