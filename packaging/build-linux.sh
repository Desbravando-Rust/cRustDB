#!/usr/bin/env bash
# Builds into target/dist/: crustdb-<version>-x86_64.{AppImage,deb,rpm,flatpak} and
# crustdb-<version>-x86_64-linux.tar.gz. Needs cargo-deb and cargo-generate-rpm; the
# .flatpak is skipped when flatpak-builder is missing.
# Usage: packaging/build-linux.sh [version]   (default: version of crates/crustdb-app)
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

# Debian and RPM packages (metadata in crates/crustdb-app/Cargo.toml).
cargo deb -p crustdb-app --no-build --deb-version "$version" -o "$dist/crustdb-$version-x86_64.deb"
cargo generate-rpm -p crates/crustdb-app -s "version = \"$version\"" -o "$dist/crustdb-$version-x86_64.rpm"

# Flatpak bundle (runtime org.freedesktop.Platform must be installed).
app_id="io.github.desbravando_rust.crustdb"
if command -v flatpak-builder >/dev/null; then
  flatpak-builder --force-clean --disable-rofiles-fuse --state-dir="$work/flatpak-state" \
    --repo="$work/flatpak-repo" "$work/flatpak-build" "packaging/flatpak/$app_id.yml"
  flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    "$work/flatpak-repo" "$dist/crustdb-$version-x86_64.flatpak" "$app_id"
else
  echo "warning: flatpak-builder not found, skipping .flatpak" >&2
fi

ls -l "$dist"
