#!/usr/bin/env bash

set -euo pipefail

fail() {
    echo "package-release: $*" >&2
    exit 1
}

sha256_file() {
    local path="$1"

    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$path" | awk '{print $1}'
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$path" | awk '{print $NF}'
    else
        fail "need sha256sum, shasum, or openssl"
    fi
}

BINARY=""
TARGET=""
OUTPUT_ROOT=""
VERSION=""
SKIP_BINARY_VERSION_CHECK=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --binary)
            [ "$#" -ge 2 ] || fail "--binary requires a value"
            BINARY="$2"
            shift 2
            ;;
        --target)
            [ "$#" -ge 2 ] || fail "--target requires a value"
            TARGET="$2"
            shift 2
            ;;
        --output-root)
            [ "$#" -ge 2 ] || fail "--output-root requires a value"
            OUTPUT_ROOT="$2"
            shift 2
            ;;
        --version)
            [ "$#" -ge 2 ] || fail "--version requires a value"
            VERSION="${2#v}"
            shift 2
            ;;
        --skip-binary-version-check)
            SKIP_BINARY_VERSION_CHECK=1
            shift
            ;;
        -h|--help)
            echo "Usage: scripts/package-release.sh --binary PATH --target TARGET --output-root DIR [--version VERSION] [--skip-binary-version-check]"
            exit 0
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
done

[ -n "$BINARY" ] || fail "--binary is required"
[ -n "$TARGET" ] || fail "--target is required"
[ -n "$OUTPUT_ROOT" ] || fail "--output-root is required"
[ -f "$BINARY" ] || fail "binary does not exist: $BINARY"
[ -x "$BINARY" ] || fail "binary is not executable: $BINARY"

case "$TARGET" in
    *[!A-Za-z0-9._-]*|'') fail "invalid target: $TARGET" ;;
esac

if [ "$SKIP_BINARY_VERSION_CHECK" -eq 1 ]; then
    [ -n "$VERSION" ] || fail "--skip-binary-version-check requires --version"
else
    BINARY_VERSION_OUTPUT="$("$BINARY" --version)"
    case "$BINARY_VERSION_OUTPUT" in
        "agent-session-facts "*) BINARY_VERSION="${BINARY_VERSION_OUTPUT#agent-session-facts }" ;;
        *) fail "unexpected --version output: $BINARY_VERSION_OUTPUT" ;;
    esac
    [ -n "$VERSION" ] || VERSION="$BINARY_VERSION"
    [ "$BINARY_VERSION" = "$VERSION" ] || fail "binary reports $BINARY_VERSION, expected $VERSION"
fi

case "$VERSION" in
    *[!0-9A-Za-z.+-]*|'') fail "invalid version: $VERSION" ;;
esac

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
[ -f "$REPO_ROOT/README.md" ] || fail "missing README.md"
[ -f "$REPO_ROOT/LICENSE" ] || fail "missing LICENSE"

ARCHIVE_NAME="agent-session-facts-v$VERSION-$TARGET.tar.gz"
ARCHIVE="$OUTPUT_ROOT/$ARCHIVE_NAME"
CHECKSUM="$ARCHIVE.sha256"
STAGING_NAME="agent-session-facts-v$VERSION-$TARGET"
STAGING_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/agent-session-facts-package.XXXXXX")"
trap 'rm -rf "$STAGING_ROOT"' EXIT

mkdir -p "$OUTPUT_ROOT" "$STAGING_ROOT/$STAGING_NAME"
install -m 0755 "$BINARY" "$STAGING_ROOT/$STAGING_NAME/agent-session-facts"
install -m 0644 "$REPO_ROOT/README.md" "$STAGING_ROOT/$STAGING_NAME/README.md"
install -m 0644 "$REPO_ROOT/LICENSE" "$STAGING_ROOT/$STAGING_NAME/LICENSE"

rm -f "$ARCHIVE" "$CHECKSUM"
COPYFILE_DISABLE=1 tar -C "$STAGING_ROOT" -cf - "$STAGING_NAME" | gzip -n > "$ARCHIVE"

HASH="$(sha256_file "$ARCHIVE" | tr 'A-F' 'a-f')"
printf '%s  %s\n' "$HASH" "$ARCHIVE_NAME" > "$CHECKSUM"

echo "$ARCHIVE"
