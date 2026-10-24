#!/usr/bin/env bash
#
# Fail unless the release tag matches the workspace version.
#
# Usage:
#   ./scripts/check-release-version.sh v0.1.0
#   GITHUB_REF_NAME=v0.1.0 ./scripts/check-release-version.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

TAG="${1:-${GITHUB_REF_NAME:-}}"
if [ -z "$TAG" ]; then
    echo "usage: $0 vMAJOR.MINOR.PATCH" >&2
    echo "       or set GITHUB_REF_NAME" >&2
    exit 2
fi

if ! [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "error: release tag '$TAG' is not vMAJOR.MINOR.PATCH" >&2
    exit 1
fi

VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml \
    | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -n1)"
if [ -z "$VERSION" ]; then
    echo "error: could not read the workspace version from Cargo.toml" >&2
    exit 1
fi

TAG_VERSION="${TAG#v}"
if [ "$TAG_VERSION" != "$VERSION" ]; then
    echo "error: release tag '$TAG' does not match workspace version '$VERSION'" >&2
    echo "       update [workspace.package] version in Cargo.toml before tagging" >&2
    exit 1
fi

echo "release tag $TAG matches workspace version $VERSION"
