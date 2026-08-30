#!/usr/bin/env bash

set -euo pipefail

fail() {
    echo "assemble-release: $*" >&2
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

DIST_ROOT=""
VERSION=""
BASE_URL=""
REQUIRED_TARGETS=()

while [ "$#" -gt 0 ]; do
    case "$1" in
        --dist-root)
            [ "$#" -ge 2 ] || fail "--dist-root requires a value"
            DIST_ROOT="$2"
            shift 2
            ;;
        --version)
            [ "$#" -ge 2 ] || fail "--version requires a value"
            VERSION="${2#v}"
            shift 2
            ;;
        --base-url)
            [ "$#" -ge 2 ] || fail "--base-url requires a value"
            BASE_URL="${2%/}"
            shift 2
            ;;
        --require-target)
            [ "$#" -ge 2 ] || fail "--require-target requires a value"
            REQUIRED_TARGETS+=("$2")
            shift 2
            ;;
        -h|--help)
            echo "Usage: scripts/assemble-release.sh --dist-root DIR --version VERSION [--base-url URL] [--require-target TARGET]..."
            exit 0
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
done

[ -n "$DIST_ROOT" ] || fail "--dist-root is required"
[ -n "$VERSION" ] || fail "--version is required"
case "$VERSION" in
    *[!0-9A-Za-z.+-]*) fail "invalid version: $VERSION" ;;
esac
if [ -n "$BASE_URL" ]; then
    case "$BASE_URL" in
        https://*|http://127.0.0.1:*|http://localhost:*) ;;
        *) fail "--base-url must use HTTPS (localhost HTTP is allowed for tests)" ;;
    esac
    case "$BASE_URL" in
        *"'"*|*$'\n'*) fail "--base-url contains unsupported characters" ;;
    esac
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
shopt -s nullglob
ARCHIVES=("$DIST_ROOT"/agent-session-facts-v"$VERSION"-*.tar.gz)
[ "${#ARCHIVES[@]}" -gt 0 ] || fail "no release archives found in $DIST_ROOT"

for archive in "${ARCHIVES[@]}"; do
    name="$(basename "$archive")"
    checksum="$archive.sha256"
    [ -f "$checksum" ] || fail "missing checksum: $checksum"
    expected_hash="$(awk 'NR == 1 { print $1 }' "$checksum" | tr 'A-F' 'a-f')"
    expected_name="$(awk 'NR == 1 { print $2 }' "$checksum")"
    [ "$expected_name" = "$name" ] || fail "$checksum names $expected_name"
    actual_hash="$(sha256_file "$archive" | tr 'A-F' 'a-f')"
    [ "$actual_hash" = "$expected_hash" ] || fail "checksum mismatch: $archive"
done

for target in "${REQUIRED_TARGETS[@]}"; do
    [ -f "$DIST_ROOT/agent-session-facts-v$VERSION-$target.tar.gz" ] ||
        fail "required target is missing: $target"
done

INSTALLER_TMP="$DIST_ROOT/.install.sh.$$"
if [ -n "$BASE_URL" ]; then
    {
        sed -n '1p' "$REPO_ROOT/scripts/install.sh"
        printf "AGENT_SESSION_FACTS_DIST_BASE_URL_DEFAULT='%s'\n" "$BASE_URL"
        sed -n '2,$p' "$REPO_ROOT/scripts/install.sh"
    } > "$INSTALLER_TMP"
else
    cp "$REPO_ROOT/scripts/install.sh" "$INSTALLER_TMP"
fi
chmod 0755 "$INSTALLER_TMP"
mv -f "$INSTALLER_TMP" "$DIST_ROOT/install.sh"
printf 'v%s\n' "$VERSION" > "$DIST_ROOT/latest"

INSTALLER_HASH="$(sha256_file "$DIST_ROOT/install.sh" | tr 'A-F' 'a-f')"
MANIFEST_TMP="$DIST_ROOT/.manifest.json.$$"
{
    printf '{\n'
    printf '  "schema_version": 1,\n'
    printf '  "version": "%s",\n' "$VERSION"
    printf '  "installer": {"path": "install.sh", "sha256": "%s"},\n' "$INSTALLER_HASH"
    printf '  "artifacts": [\n'
    for i in "${!ARCHIVES[@]}"; do
        archive="${ARCHIVES[$i]}"
        name="$(basename "$archive")"
        target="${name#agent-session-facts-v$VERSION-}"
        target="${target%.tar.gz}"
        hash="$(awk 'NR == 1 { print $1 }' "$archive.sha256" | tr 'A-F' 'a-f')"
        comma=','
        [ "$i" -eq "$((${#ARCHIVES[@]} - 1))" ] && comma=''
        printf '    {"target": "%s", "archive": "%s", "sha256": "%s"}%s\n' \
            "$target" "$name" "$hash" "$comma"
    done
    printf '  ]\n'
    printf '}\n'
} > "$MANIFEST_TMP"
mv -f "$MANIFEST_TMP" "$DIST_ROOT/manifest.json"

echo "$DIST_ROOT"
