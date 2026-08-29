# AGENTS.md

## Product Boundary

`agent-session-facts` is an independent read-only CLI that extracts auditable facts from native Agent runtime Session artifacts. It does not belong to RunLab, does not depend on RunLab or Trace Index, and does not judge task quality.

Keep runtime-specific parsing behind explicit adapters. A new adapter must be justified by real artifacts and must preserve native identities, exact source provenance, incomplete evidence, and unsupported transitions rather than guessing.

## Rust Implementation

- Maintain one Rust 2024 workspace with one `agent-session-facts` binary package.
- Keep `rust-toolchain.toml` fixed and `package.rust-version` as the tested MSRV.
- `unsafe` is forbidden. Prefer blocking standard-library I/O and bounded streaming over an async runtime.
- Keep stdout to one compact JSON report and stderr to one compact JSON error. Help, exit statuses, and incomplete-report semantics are part of the public Agent-facing contract.
- Separate recorded execution facts from inferred judgments. Never estimate model cost when the runtime artifact does not record the required facts.

## Verification

Before claiming completion, run formatting, all-target tests, Clippy with warnings denied, the declared MSRV check, packaging, and separate-process CLI tests. Validate supported adapters against representative real artifacts, including single-file and multi-rollout Sessions, and independently cross-check accounting when its semantics change.
