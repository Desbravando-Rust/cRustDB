#!/usr/bin/env bash
# Release em dois passos (nunca faz push na main):
#   packaging/release.sh bump  -> pergunta major/minor/patch, ajusta crates/crustdb-app/Cargo.toml,
#                               commita em release/vX.Y.Z e abre o PR para a main.
#   packaging/release.sh tag   -> após o merge, cria a tag vX.Y.Z na main e envia ao remote,
#                               disparando o job release do CI (instaladores no GitHub Release).
set -euo pipefail

cd "$(dirname "$0")/.."
remote="${REMOTE:-origin}"
manifest="crates/crustdb-app/Cargo.toml"

current_version() { awk -F'"' '/^version = / {print $2; exit}' "$manifest"; }

require_clean() {
  if [ -n "$(git status --porcelain)" ]; then
    echo "❌ Working tree suja. Commite ou descarte as mudanças antes."
    exit 1
  fi
}

confirm() {
  read -rp "$1 [s/N] " answer
  [[ "$answer" =~ ^[sSyY]$ ]] || { echo "Cancelado."; exit 1; }
}

bump() {
  require_clean
  git fetch -q "$remote" main
  git switch -q --detach "$remote/main"

  local cur major minor patch kind next
  cur="$(current_version)"
  [[ "$cur" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || { echo "❌ Versão inválida em $manifest: '$cur'"; exit 1; }
  major="${BASH_REMATCH[1]}" minor="${BASH_REMATCH[2]}" patch="${BASH_REMATCH[3]}"

  echo "Versão atual: $cur"
  read -rp "Tipo de bump (major/minor/patch): " kind
  case "$kind" in
    major) next="$((major + 1)).0.0" ;;
    minor) next="$major.$((minor + 1)).0" ;;
    patch) next="$major.$minor.$((patch + 1))" ;;
    *) echo "❌ Use major, minor ou patch."; exit 1 ;;
  esac

  if git ls-remote --exit-code --tags "$remote" "refs/tags/v$next" >/dev/null; then
    echo "❌ Tag v$next já existe em $remote."
    exit 1
  fi
  confirm "Bump $cur -> $next e abrir PR?"

  git switch -q -c "release/v$next"
  sed -i "0,/^version = \"$cur\"/s//version = \"$next\"/" "$manifest"
  # Atualiza a versão do crate no Cargo.lock sem compilar.
  cargo metadata --format-version 1 >/dev/null
  git add "$manifest" Cargo.lock
  git commit -q -m "chore(release): v$next"
  git push -q -u "$remote" "release/v$next"
  gh pr create --base main --title "chore(release): v$next" \
    --body "Bump crustdb-app to $next. After merge, run \`make release-tag\` to tag v$next and build the installers."
}

tag() {
  require_clean
  git switch -q main
  git pull -q --ff-only "$remote" main

  local version
  version="$(current_version)"
  if git rev-parse -q --verify "refs/tags/v$version" >/dev/null \
    || git ls-remote --exit-code --tags "$remote" "refs/tags/v$version" >/dev/null; then
    echo "❌ Tag v$version já existe. Rode 'make release' primeiro."
    exit 1
  fi
  confirm "Criar e enviar tag v$version ($(git rev-parse --short HEAD)) para $remote?"

  git tag -a "v$version" -m "v$version"
  git push -q "$remote" "v$version"
  echo "✅ Tag v$version enviada. Acompanhe o job release em: gh run list --workflow ci.yml"
}

case "${1:-}" in
  bump) bump ;;
  tag) tag ;;
  *) echo "Uso: $0 bump|tag"; exit 1 ;;
esac
