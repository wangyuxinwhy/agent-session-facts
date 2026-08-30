#!/bin/sh

set -eu

AGENT_SESSION_FACTS_DIST_BASE_URL_DEFAULT="${AGENT_SESSION_FACTS_DIST_BASE_URL_DEFAULT:-https://github.com/wangyuxinwhy/agent-session-facts/releases}"

usage() {
    cat <<'EOF'
Usage: install.sh [options]

Install a prebuilt agent-session-facts binary without Cargo.

Options:
  --version VERSION   Version to install (default: latest)
  --bin-dir DIR       Destination directory (default: ~/.local/bin)
  --base-url URL      GitHub Releases root; overrides the published default
  --dry-run           Print the resolved download without changing anything
  --allow-http        Permit plain HTTP (intended only for local testing)
  -h, --help          Show this help
EOF
}

fail() {
    echo "agent-session-facts installer: $*" >&2
    exit 1
}

sha256_file() {
    path="$1"

    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$path" | awk '{print $1}'
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$path" | awk '{print $NF}'
    else
        fail "need sha256sum, shasum, or openssl to verify the download"
    fi
}

fetch() {
    url="$1"
    output="$2"

    command -v curl >/dev/null 2>&1 || fail "curl is required"
    if [ "$ALLOW_HTTP" -eq 1 ]; then
        curl --proto '=http,https' --proto-redir '=http,https' \
            --fail --location --silent --show-error \
            --output "$output" "$url"
    else
        curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
            --fail --location --silent --show-error \
            --output "$output" "$url"
    fi
}

VERSION="latest"
BASE_URL="${AGENT_SESSION_FACTS_DIST_BASE_URL:-$AGENT_SESSION_FACTS_DIST_BASE_URL_DEFAULT}"
BIN_DIR="${AGENT_SESSION_FACTS_BIN_DIR:-${HOME:?HOME is not set}/.local/bin}"
DRY_RUN=0
ALLOW_HTTP=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version requires a value"
            VERSION="$2"
            shift 2
            ;;
        --bin-dir)
            [ "$#" -ge 2 ] || fail "--bin-dir requires a value"
            BIN_DIR="$2"
            shift 2
            ;;
        --base-url)
            [ "$#" -ge 2 ] || fail "--base-url requires a value"
            BASE_URL="$2"
            shift 2
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --allow-http)
            ALLOW_HTTP=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
done

[ -n "$BASE_URL" ] || fail "no distribution URL is published; pass --base-url URL"
BASE_URL="${BASE_URL%/}"

case "$BASE_URL" in
    https://*) ;;
    http://*) [ "$ALLOW_HTTP" -eq 1 ] || fail "plain HTTP requires --allow-http" ;;
    *) fail "--base-url must use HTTPS" ;;
esac

case "$BIN_DIR" in
    /*) ;;
    *) fail "--bin-dir must be an absolute path" ;;
esac

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS:$ARCH" in
    Darwin:arm64|Darwin:aarch64) TARGET="aarch64-apple-darwin" ;;
    Darwin:x86_64) TARGET="x86_64-apple-darwin" ;;
    Linux:x86_64|Linux:amd64) TARGET="x86_64-unknown-linux-gnu" ;;
    Linux:arm64|Linux:aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
    *) fail "unsupported platform: $OS $ARCH" ;;
esac

if [ "$DRY_RUN" -eq 1 ]; then
    if [ "$VERSION" = "latest" ]; then
        echo "version source: $BASE_URL/latest/download/latest"
        echo "artifact target: $TARGET"
    else
        VERSION="${VERSION#v}"
        echo "artifact: $BASE_URL/download/v$VERSION/agent-session-facts-v$VERSION-$TARGET.tar.gz"
    fi
    echo "destination: $BIN_DIR/agent-session-facts"
    exit 0
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/agent-session-facts-install.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT HUP INT TERM

if [ "$VERSION" = "latest" ]; then
    echo "==> Resolving the latest agent-session-facts version" >&2
    fetch "$BASE_URL/latest/download/latest" "$TMP_DIR/latest"
    VERSION="$(tr -d '\r\n' < "$TMP_DIR/latest")"
fi
VERSION="${VERSION#v}"

printf '%s\n' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.+-][0-9A-Za-z.-]+)?$' ||
    fail "invalid release version: $VERSION"

ARCHIVE_NAME="agent-session-facts-v$VERSION-$TARGET.tar.gz"
ARCHIVE_URL="$BASE_URL/download/v$VERSION/$ARCHIVE_NAME"
CHECKSUM_URL="$ARCHIVE_URL.sha256"
ARCHIVE="$TMP_DIR/$ARCHIVE_NAME"
CHECKSUM="$ARCHIVE.sha256"
STAGING_NAME="agent-session-facts-v$VERSION-$TARGET"

echo "==> Downloading $ARCHIVE_NAME" >&2
fetch "$ARCHIVE_URL" "$ARCHIVE"
fetch "$CHECKSUM_URL" "$CHECKSUM"

EXPECTED_HASH="$(awk 'NR == 1 { print $1 }' "$CHECKSUM")"
EXPECTED_NAME="$(awk 'NR == 1 { print $2 }' "$CHECKSUM")"
[ "$EXPECTED_NAME" = "$ARCHIVE_NAME" ] || fail "checksum names $EXPECTED_NAME, expected $ARCHIVE_NAME"
[ "${#EXPECTED_HASH}" -eq 64 ] || fail "checksum is not a SHA-256 digest"
case "$EXPECTED_HASH" in
    *[!0-9A-Fa-f]*) fail "checksum is not a SHA-256 digest" ;;
esac

ACTUAL_HASH="$(sha256_file "$ARCHIVE" | tr 'A-F' 'a-f')"
EXPECTED_HASH="$(printf '%s' "$EXPECTED_HASH" | tr 'A-F' 'a-f')"
[ "$ACTUAL_HASH" = "$EXPECTED_HASH" ] || fail "SHA-256 mismatch for $ARCHIVE_NAME"

ENTRIES="$(tar -tzf "$ARCHIVE" | LC_ALL=C sort)"
EXPECTED_ENTRIES="$(printf '%s\n' \
    "$STAGING_NAME/" \
    "$STAGING_NAME/LICENSE" \
    "$STAGING_NAME/README.md" \
    "$STAGING_NAME/agent-session-facts" | LC_ALL=C sort)"
[ "$ENTRIES" = "$EXPECTED_ENTRIES" ] || fail "archive contains unexpected paths"

tar -xzf "$ARCHIVE" -C "$TMP_DIR" \
    "$STAGING_NAME/LICENSE" \
    "$STAGING_NAME/README.md" \
    "$STAGING_NAME/agent-session-facts"
BINARY="$TMP_DIR/$STAGING_NAME/agent-session-facts"
[ -x "$BINARY" ] || fail "archive did not contain an executable agent-session-facts"

REPORTED_VERSION="$("$BINARY" --version)"
[ "$REPORTED_VERSION" = "agent-session-facts $VERSION" ] ||
    fail "downloaded binary reports '$REPORTED_VERSION', expected 'agent-session-facts $VERSION'"

mkdir -p "$BIN_DIR"
INSTALL_TMP="$BIN_DIR/.agent-session-facts.install.$$"
trap 'rm -rf "$TMP_DIR"; rm -f "$INSTALL_TMP"' EXIT HUP INT TERM
install -m 0755 "$BINARY" "$INSTALL_TMP"
mv -f "$INSTALL_TMP" "$BIN_DIR/agent-session-facts"

echo "installed agent-session-facts $VERSION to $BIN_DIR/agent-session-facts"
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "add $BIN_DIR to PATH before running agent-session-facts" >&2 ;;
esac
