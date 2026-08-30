#!/usr/bin/env bash

set -euo pipefail

fail() {
    echo "test-installer: $*" >&2
    exit 1
}

for tool in cargo rustc python3 curl tar; do
    command -v "$tool" >/dev/null 2>&1 || fail "missing required tool: $tool"
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/agent-session-facts-installer-test.XXXXXX")"
SERVER_PID=""

cleanup() {
    if [ -n "$SERVER_PID" ]; then
        kill "$SERVER_PID" >/dev/null 2>&1 || true
        wait "$SERVER_PID" >/dev/null 2>&1 || true
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT HUP INT TERM

TARGET="$(rustc -vV | sed -n 's/^host: //p')"
[ -n "$TARGET" ] || fail "could not determine the host target"

if "$REPO_ROOT/scripts/build-release-artifact.sh" \
    --target "$TARGET" --version 999.0.0 --output-root "$WORK/tag-mismatch" \
    >"$WORK/tag-mismatch.stdout" 2>"$WORK/tag-mismatch.stderr"; then
    fail "release build accepted a tag/package version mismatch"
fi
grep -q 'does not match Cargo package version' "$WORK/tag-mismatch.stderr" ||
    fail "tag/package mismatch did not explain the failure"

ARCHIVE="$("$REPO_ROOT/scripts/build-release-artifact.sh" \
    --target "$TARGET" --output-root "$WORK/dist")"
VERSION="$(basename "$ARCHIVE")"
VERSION="${VERSION#agent-session-facts-v}"
VERSION="${VERSION%-$TARGET.tar.gz}"

mkdir -p "$WORK/server"
PORT_FILE="$WORK/port"
python3 - "$WORK/server" "$PORT_FILE" >"$WORK/server.log" 2>&1 <<'PY' &
import http.server
import pathlib
import socketserver
import sys

root = pathlib.Path(sys.argv[1])
port_file = pathlib.Path(sys.argv[2])
handler = lambda *args, **kwargs: http.server.SimpleHTTPRequestHandler(
    *args, directory=str(root), **kwargs
)
with socketserver.TCPServer(("127.0.0.1", 0), handler) as server:
    port_file.write_text(str(server.server_address[1]), encoding="utf-8")
    server.serve_forever()
PY
SERVER_PID=$!

attempt=0
while [ "$attempt" -lt 100 ] && [ ! -s "$PORT_FILE" ]; do
    kill -0 "$SERVER_PID" >/dev/null 2>&1 || fail "HTTP server exited before becoming ready"
    sleep 0.05
    attempt=$((attempt + 1))
done
[ -s "$PORT_FILE" ] || fail "HTTP server did not become ready"
BASE_URL="http://127.0.0.1:$(cat "$PORT_FILE")/releases"

"$REPO_ROOT/scripts/assemble-release.sh" \
    --dist-root "$WORK/dist" \
    --version "$VERSION" \
    --base-url "$BASE_URL" \
    --require-target "$TARGET" >/dev/null

mkdir -p \
    "$WORK/server/releases/latest/download" \
    "$WORK/server/releases/download/v$VERSION"
cp "$WORK/dist/install.sh" "$WORK/server/releases/latest/download/install.sh"
cp "$WORK/dist/latest" "$WORK/server/releases/latest/download/latest"
cp "$WORK/dist/agent-session-facts-v$VERSION-$TARGET.tar.gz"* \
    "$WORK/server/releases/download/v$VERSION/"

python3 - "$WORK/dist/manifest.json" "$VERSION" "$TARGET" <<'PY'
import json
import pathlib
import sys

manifest = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
assert manifest["schema_version"] == 1
assert manifest["version"] == sys.argv[2]
assert [artifact["target"] for artifact in manifest["artifacts"]] == [sys.argv[3]]
PY

curl --fail --silent --show-error \
    "$BASE_URL/latest/download/install.sh" \
    --output "$WORK/downloaded-install.sh"
chmod +x "$WORK/downloaded-install.sh"

DRY_RUN_OUTPUT="$("$WORK/downloaded-install.sh" --allow-http \
    --version "$VERSION" --bin-dir "$WORK/dry-run-bin" --dry-run)"
printf '%s\n' "$DRY_RUN_OUTPUT" | grep -q \
    "releases/download/v$VERSION/agent-session-facts-v$VERSION-$TARGET.tar.gz" ||
    fail "dry-run did not resolve the expected GitHub Release asset"
[ ! -e "$WORK/dry-run-bin" ] || fail "dry-run changed the filesystem"

INSTALL_OUTPUT="$("$WORK/downloaded-install.sh" --allow-http --bin-dir "$WORK/bin")"
printf '%s\n' "$INSTALL_OUTPUT" | grep -q "installed agent-session-facts $VERSION" ||
    fail "installer did not report success"
[ "$("$WORK/bin/agent-session-facts" --version)" = "agent-session-facts $VERSION" ] ||
    fail "installed binary reported the wrong version"

CHECKSUM="$WORK/server/releases/download/v$VERSION/agent-session-facts-v$VERSION-$TARGET.tar.gz.sha256"
printf '%064d  agent-session-facts-v%s-%s.tar.gz\n' 0 "$VERSION" "$TARGET" > "$CHECKSUM"
if "$WORK/downloaded-install.sh" --allow-http --bin-dir "$WORK/corrupt-bin" \
    >"$WORK/corrupt.stdout" 2>"$WORK/corrupt.stderr"; then
    fail "installer accepted a corrupt checksum"
fi
grep -q 'SHA-256 mismatch' "$WORK/corrupt.stderr" || fail "checksum failure was not explained"
[ ! -e "$WORK/corrupt-bin/agent-session-facts" ] || fail "checksum failure installed a binary"

echo "installer test passed for agent-session-facts $VERSION ($TARGET)"
