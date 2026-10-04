#!/bin/sh
# Run from an extracted Terminal Workspace archive. No root privileges required.
set -eu

package_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
prefix=${1:-"$HOME/.local"}
if [ "$#" -gt 1 ]; then
    echo "Usage: sh install.sh [prefix]" >&2
    exit 1
fi
case "$(uname -s)/$(uname -m)" in
    Darwin/arm64) target=aarch64-apple-darwin ;;
    Darwin/x86_64) target=x86_64-apple-darwin ;;
    Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu ;;
    Linux/x86_64) target=x86_64-unknown-linux-gnu ;;
    *) echo "Unsupported OS/architecture" >&2; exit 1 ;;
esac
if [ "$(cat "$package_dir/TARGET")" != "$target" ]; then
    echo "This archive does not match your OS/architecture ($target)" >&2
    exit 1
fi
mkdir -p "$prefix/bin"
install -m 755 "$package_dir/tw" "$prefix/bin/tw"
echo "Installed: $prefix/bin/tw"
echo "Add $prefix/bin to PATH, then run: tw /path/to/project"
