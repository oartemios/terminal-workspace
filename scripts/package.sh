#!/bin/sh
# Package an already built native release binary, preserving executable mode.
set -eu
if [ "$#" -ne 1 ]; then
    echo "Usage: sh scripts/package.sh <Rust target triple>" >&2
    exit 1
fi
target=$1
host=$(rustc -vV | sed -n 's/^host: //p')
if [ "$target" != "$host" ]; then
    echo "Expected native target $host, received $target" >&2
    exit 1
fi
case "$target" in
    aarch64-apple-darwin|x86_64-apple-darwin|aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu) ;;
    *) echo "Unsupported package target: $target" >&2; exit 1 ;;
esac
metadata=$(python3 scripts/version.py)
version=$(printf '%s' "$metadata" | python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])')
build_version=$(printf '%s' "$metadata" | python3 -c 'import json,sys; print(json.load(sys.stdin)["build_version"])')
if [ "$(target/release/tw --version)" != "tw $version" ]; then
    echo "Release binary version differs from Cargo.toml; rebuild before packaging" >&2
    exit 1
fi
name="terminal-workspace-$build_version-$target"
mkdir -p "dist/$name"
install -m 755 target/release/tw "dist/$name/tw"
cp scripts/install.sh README.md "dist/$name/"
printf '%s\n' "$target" > "dist/$name/TARGET"
printf '%s\n' "$version" > "dist/$name/VERSION"
printf '%s\n' "$build_version" > "dist/$name/BUILD_VERSION"
git rev-parse HEAD > "dist/$name/COMMIT"
tar -czf "dist/$name.tar.gz" -C dist "$name"
(cd dist && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256")
echo "Created dist/$name.tar.gz"
