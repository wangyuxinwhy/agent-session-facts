#!/usr/bin/env bash

set -euo pipefail

fail() {
    echo "build-release-artifact: $*" >&2
    exit 1
}

TARGET=""
OUTPUT_ROOT=""
VERSION=""

while [ "$#" -gt 0 ]; do
    case "$1" in
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
        -h|--help)
            echo "Usage: scripts/build-release-artifact.sh [--target TARGET] --output-root DIR [--version VERSION]"
            exit 0
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
done

[ -n "$OUTPUT_ROOT" ] || fail "--output-root is required"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

[ -n "$TARGET" ] || TARGET="$(rustc -vV | sed -n 's/^host: //p')"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
[ -n "$TARGET" ] || fail "could not determine the Rust target"
[ -n "$HOST_TARGET" ] || fail "could not determine the Rust host target"

PACKAGE_ID="$(cargo pkgid)"
case "$PACKAGE_ID" in
    *'#'*) PACKAGE_VERSION="${PACKAGE_ID##*#}" ;;
    *@*) PACKAGE_VERSION="${PACKAGE_ID##*@}" ;;
    *) fail "could not read a version from cargo pkgid: $PACKAGE_ID" ;;
esac
[ -n "$VERSION" ] || VERSION="$PACKAGE_VERSION"
[ "$VERSION" = "$PACKAGE_VERSION" ] ||
    fail "requested version $VERSION does not match Cargo package version $PACKAGE_VERSION"

echo "==> Building agent-session-facts for $TARGET" >&2
cargo build --release --locked --target "$TARGET" --bin agent-session-facts

TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
case "$TARGET_DIR" in
    /*) ;;
    *) TARGET_DIR="$REPO_ROOT/$TARGET_DIR" ;;
esac

ARGS=(
    --binary "$TARGET_DIR/$TARGET/release/agent-session-facts"
    --target "$TARGET"
    --output-root "$OUTPUT_ROOT"
    --version "$VERSION"
)
if [ "$TARGET" != "$HOST_TARGET" ]; then
    ARGS+=(--skip-binary-version-check)
fi

"$REPO_ROOT/scripts/package-release.sh" "${ARGS[@]}"
