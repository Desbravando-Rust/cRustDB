#!/usr/bin/env bash
# Builds target/dist/crustdb-<version>-x86_64.AppImage and dist/crustdb-<version>-x86_64-linux.tar.gz.
# Usage: packaging/build-appimage.sh [version]   (default: version of crates/crustdb-app)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

version="${1:-$(awk -F'"' '/^version/ {print $2; exit}' crates/crustdb-app/Cargo.toml)}"
dist="$root/target/dist"
work="$root/target/packaging"
linuxdeploy_url="https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage"
linuxdeploy_sha256="c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d"

cargo build --release --locked -p crustdb-app

mkdir -p "$dist" "$work"
tool="$work/linuxdeploy-x86_64.AppImage"
if ! echo "$linuxdeploy_sha256  $tool" | sha256sum --check --status 2>/dev/null; then
  curl -fsSL -o "$tool" "$linuxdeploy_url"
  echo "$linuxdeploy_sha256  $tool" | sha256sum --check --status
  chmod +x "$tool"
fi

# AppImage: no FUSE needed to run linuxdeploy itself (containers, CI).
rm -rf "$work/AppDir"
(cd "$work" && APPIMAGE_EXTRACT_AND_RUN=1 OUTPUT="crustdb-$version-x86_64.AppImage" "$tool" \
  --appdir AppDir \
  --executable "$root/target/release/crustdb" \
  --desktop-file "$root/packaging/crustdb.desktop" \
  --icon-file "$root/packaging/crustdb.png" \
  --output appimage)
mv "$work/crustdb-$version-x86_64.AppImage" "$dist/"

# Tarball: binary + desktop entry + icon.
name="crustdb-$version-x86_64-linux"
rm -rf "${work:?}/$name"
mkdir -p "$work/$name"
cp target/release/crustdb packaging/crustdb.desktop packaging/crustdb.png "$work/$name/"
tar -C "$work" -czf "$dist/$name.tar.gz" "$name"

ls -l "$dist"
