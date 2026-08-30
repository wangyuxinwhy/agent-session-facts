# Releasing agent-session-facts

Publishing source, a crate, a Git tag, GitHub Release assets, and documentation are separate state changes. Do not treat a successful build as evidence that the public installation path works.

## Prepare

1. Update `Cargo.toml`, `Cargo.lock`, README and documentation installation examples to the same version.
2. Run the complete local verification:

   ```bash
   cargo fmt --all -- --check
   cargo test --all-targets --locked
   cargo clippy --all-targets --all-features --locked -- -D warnings
   cargo +1.95 test --all-targets --locked
   cargo package --locked
   cargo publish --dry-run --locked
   npm ci
   npm run docs:build
   npm run docs:check-agent
   ./scripts/test-installer.sh
   ```

3. Inspect `cargo package --list` and the generated installer manifest. Confirm that no credential, private path or unrelated project reference is included.
4. Commit and push the release candidate. Wait for CI and documentation deployment to succeed.

## Publish

1. Publish the crate with `cargo publish --locked`.
2. Create and push an annotated `vVERSION` tag that exactly matches `Cargo.toml`.
3. Wait for the Release workflow. It builds four native archives, verifies their reported versions, checks every checksum, and attaches `install.sh`, `latest`, and `manifest.json`.
4. Verify both public installation paths from the registry and Release assets:

   ```bash
   cargo install agent-session-facts --version VERSION --locked
   curl --proto '=https' --tlsv1.2 -fsSL \
     'https://github.com/wangyuxinwhy/agent-session-facts/releases/latest/download/install.sh' | sh
   ```

5. Verify the GitHub Pages HTML, `llms.txt`, `llms-full.txt`, and the Markdown installation page. Preserve prior tags and artifacts for auditability; clearly deprecate a defective release rather than silently replacing its history.
