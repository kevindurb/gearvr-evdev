#!/usr/bin/env bash
#
# Build the portable release tarball.
#
# Run this inside an Ubuntu 22.04 environment (or any host with pkg-config and
# libdbus-1 development headers). The resulting binary targets glibc 2.35 and
# links the system libdbus-1.so.3. The release workflow runs this script inside
# the ubuntu:22.04 container.
#
# Usage:
#   ./scripts/build-tarball.sh
#   DIST_DIR=/tmp/out ./scripts/build-tarball.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml \
    | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -n1)"
if [ -z "$VERSION" ]; then
    echo "error: could not read the workspace version from Cargo.toml" >&2
    exit 1
fi

for tool in cargo objdump readelf strip tar; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "error: required tool not found: $tool" >&2
        exit 1
    fi
done

if ! pkg-config --exists dbus-1 2>/dev/null; then
    echo "error: dbus-1 development files are required for the release build" >&2
    echo "       install libdbus-1-dev (Debian/Ubuntu) or dbus-devel (Fedora)" >&2
    exit 1
fi

DIST_DIR="${DIST_DIR:-$REPO_ROOT/dist}"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
case "$TARGET_DIR" in
    /*) ;;
    *) TARGET_DIR="$REPO_ROOT/$TARGET_DIR" ;;
esac

PKG_NAME="gearvr-evdev-${VERSION}"
STAGE_ROOT="$(mktemp -d)"
trap 'rm -rf "$STAGE_ROOT"' EXIT
STAGE="$STAGE_ROOT/$PKG_NAME"

echo "==> Building gearvr-cli ${VERSION} with the system libdbus"
cargo build --release --locked -p gearvr-cli \
    --no-default-features --features bluez,dualsense

echo "==> Staging $PKG_NAME"
install -Dm755 "$TARGET_DIR/release/gearvr-cli" "$STAGE/gearvr-cli"
strip "$STAGE/gearvr-cli"
install -Dm755 packaging/install.sh "$STAGE/install.sh"
install -Dm644 README.md "$STAGE/README.md"
install -Dm644 LICENSE "$STAGE/LICENSE"
install -Dm644 packaging/README.md "$STAGE/INSTALL.md"
install -Dm644 packaging/systemd/gearvr.service "$STAGE/systemd/gearvr.service"
install -Dm644 packaging/udev/70-gearvr-evdev.rules "$STAGE/udev/70-gearvr-evdev.rules"
install -Dm644 packaging/udev/60-gearvr-permissions.rules "$STAGE/udev/60-gearvr-permissions.rules"

echo "==> Checking the binary's portability baseline"
if ! readelf -d "$STAGE/gearvr-cli" | grep -q 'libdbus-1\.so\.3'; then
    echo "error: the release binary does not link libdbus-1.so.3" >&2
    echo "       build it with --no-default-features --features bluez,dualsense" >&2
    exit 1
fi

MAX_GLIBC="$(objdump -T "$STAGE/gearvr-cli" \
    | sed -n 's/.*(GLIBC_\([0-9][0-9.]*\)).*/\1/p' \
    | sort -Vu | tail -n1)"
if [ -z "$MAX_GLIBC" ]; then
    echo "error: could not determine the imported GLIBC versions" >&2
    exit 1
fi
HIGHEST="$(printf '%s\n' 2.35 "$MAX_GLIBC" | sort -V | tail -n1)"
if [ "$HIGHEST" != "2.35" ]; then
    echo "error: binary requires GLIBC_$MAX_GLIBC; the release baseline is GLIBC_2.35" >&2
    exit 1
fi
echo "    libdbus-1.so.3: yes"
echo "    highest imported glibc: $MAX_GLIBC"

mkdir -p "$DIST_DIR"
TARBALL="$DIST_DIR/$PKG_NAME-x86_64-linux-gnu.tar.gz"
tar -C "$STAGE_ROOT" -czf "$TARBALL" "$PKG_NAME"

echo "==> Archive contents"
tar -tzf "$TARBALL"
echo "==> Wrote $TARBALL"
sha256sum "$TARBALL"
