#!/usr/bin/env bash
# Testa `packaging/release.sh changelog <versão>` num diretório temporário.
# Uso: bash packaging/release_test.sh
set -uo pipefail

script="$(cd "$(dirname "$0")" && pwd)/release.sh"
fails=0
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
export CHANGELOG_DIR="$tmp/changelog.d" CHANGELOG_FILE="$tmp/CHANGELOG.md"
link() { echo "[#$1](https://github.com/Desbravando-Rust/cRustDB/issues/$1)"; }

# Arrange
mkdir -p "$CHANGELOG_DIR"
touch "$CHANGELOG_DIR/.gitkeep"
printf '# Changelog\n\nHeader line.\n\n## [0.1.0]\n\n### Added\n\n- old item\n' >"$CHANGELOG_FILE"
echo "- $(link 10) — New feature." >"$CHANGELOG_DIR/10.added.md"
echo "- $(link 11) — Fixed bug." >"$CHANGELOG_DIR/11.fixed.md"
echo "- $(link 12) — Another feature." >"$CHANGELOG_DIR/12.added.md"

# Act
"$script" changelog 0.2.0 >/dev/null || fail "changelog 0.2.0 retornou erro"

# Assert
expected="# Changelog

Header line.

## [0.2.0]

### Added

- $(link 10) — New feature.
- $(link 12) — Another feature.

### Fixed

- $(link 11) — Fixed bug.

## [0.1.0]

### Added

- old item"
[ "$(cat "$CHANGELOG_FILE")" = "$expected" ] || {
  fail "CHANGELOG inesperado"
  diff <(echo "$expected") "$CHANGELOG_FILE"
}
[ "$(ls -A "$CHANGELOG_DIR")" = ".gitkeep" ] || fail "changelog.d não ficou vazio"

# Act & Assert: sem fragmentos falha
"$script" changelog 0.3.0 >/dev/null 2>&1 && fail "sem fragmentos deveria falhar"

# Act & Assert: RC rejeitado
echo "- x" >"$CHANGELOG_DIR/13.added.md"
"$script" changelog 0.2.0-rc1 >/dev/null 2>&1 && fail "0.2.0-rc1 deveria ser rejeitado"
[ -f "$CHANGELOG_DIR/13.added.md" ] || fail "RC não deve consumir fragmentos"

[ "$fails" -eq 0 ] && echo "ok" || exit 1
