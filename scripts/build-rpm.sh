#!/usr/bin/env bash
#
# Build the prebuilt-binary RPM from a release tarball.
#
# Run this inside a Fedora environment with rpm-build and systemd-rpm-macros:
#
#   ./scripts/build-rpm.sh
#   ./scripts/build-rpm.sh dist/gearvr-evdev-0.1.0-x86_64-linux-gnu.tar.gz
#
# The tarball defaults to dist/gearvr-evdev-<version>-x86_64-linux-gnu.tar.gz.

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

for tool in rpmbuild tar; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "error: required tool not found: $tool" >&2
        echo "       install rpm-build (Fedora) before running this script" >&2
        exit 1
    fi
done

DIST_DIR="${DIST_DIR:-$REPO_ROOT/dist}"
TARBALL="${1:-$DIST_DIR/gearvr-evdev-${VERSION}-x86_64-linux-gnu.tar.gz}"
if [ ! -f "$TARBALL" ]; then
    echo "error: release tarball not found: $TARBALL" >&2
    echo "       run scripts/build-tarball.sh first" >&2
    exit 1
fi

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
mkdir -p "$WORK_DIR"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}

cp "$TARBALL" "$WORK_DIR/SOURCES/"
cp packaging/rpm/gearvr-evdev.spec "$WORK_DIR/SPECS/"

echo "==> Building gearvr-evdev ${VERSION}-1 RPM"
rpmbuild -bb \
    --define "_topdir $WORK_DIR" \
    --define "gearvr_version $VERSION" \
    "$WORK_DIR/SPECS/gearvr-evdev.spec"

RPM="$WORK_DIR/RPMS/x86_64/gearvr-evdev-${VERSION}-1.x86_64.rpm"
if [ ! -f "$RPM" ]; then
    echo "error: rpmbuild did not produce $RPM" >&2
    exit 1
fi

mkdir -p "$DIST_DIR"
cp "$RPM" "$DIST_DIR/"
DEST="$DIST_DIR/$(basename "$RPM")"

echo "==> Wrote $DEST"
echo "==> Requirements"
rpm -qpR "$DEST"
echo "==> Files"
rpm -qpl "$DEST"
sha256sum "$DEST"
