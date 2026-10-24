#!/usr/bin/env bash
# Release em dois passos (nunca faz push na main):
#   packaging/release.sh bump  -> pergunta major/minor/patch/current, ajusta crates/crustdb-app/Cargo.toml,
#                               gera a seção do CHANGELOG a partir de changelog.d/, commita em
#                               release/vX.Y.Z e abre o PR para a main.
#   packaging/release.sh tag   -> após o merge, cria a tag vX.Y.Z na main e envia ao remote,
#                               disparando o job release do CI (instaladores no GitHub Release).
#   packaging/release.sh changelog X.Y.Z -> só converte os fragmentos em `## [X.Y.Z]` (usado pelo bump).
set -euo pipefail

cd "$(dirname "$0")/.."
remote="${REMOTE:-origin}"
manifest="crates/crustdb-app/Cargo.toml"
changelog_dir="${CHANGELOG_DIR:-changelog.d}"
changelog_file="${CHANGELOG_FILE:-CHANGELOG.md}"

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

# Fragmentos: changelog.d/<ID>.<added|changed|fixed>.md. Falha se não houver nenhum ou se algum tiver tipo desconhecido.
check_fragments() {
  local f
  compgen -G "$changelog_dir/*.md" >/dev/null || { echo "❌ Nenhum fragmento em $changelog_dir/."; exit 1; }
  for f in "$changelog_dir"/*.md; do
    [[ "$f" =~ /[0-9]+\.(added|changed|fixed)\.md$ ]] || { echo "❌ Fragmento inválido: $f"; exit 1; }
  done
}

# Insere `## [X.Y.Z]` acima da versão anterior (abaixo do cabeçalho) e apaga os fragmentos.
changelog() {
  local version="${1:-}" body kind f n tmp
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "❌ Versão inválida (RC não gera seção): '$version'"; exit 1; }
  check_fragments

  body="## [$version]"
  for kind in added changed fixed; do
    compgen -G "$changelog_dir/*.$kind.md" >/dev/null || continue
    body+=$'\n\n'"### ${kind^}"$'\n'
    while read -r f; do body+=$'\n'"$(cat "$f")"; done < <(compgen -G "$changelog_dir/*.$kind.md" | sort -V)
  done

  tmp="$(mktemp)"
  n="$(grep -n -m1 '^## ' "$changelog_file" | cut -d: -f1 || true)"
  if [ -n "$n" ]; then
    { head -n "$((n - 1))" "$changelog_file"; printf '%s\n\n' "$body"; tail -n +"$n" "$changelog_file"; } >"$tmp"
  else
    { cat "$changelog_file"; printf '\n%s\n' "$body"; } >"$tmp"
  fi
  mv "$tmp" "$changelog_file"
  rm -f "$changelog_dir"/*.md
  echo "✅ $changelog_file: seção [$version] gerada."
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
  check_fragments
  read -rp "Tipo de bump (major/minor/patch/current): " kind
  case "$kind" in
    current) next="$cur" ;;
    major) next="$((major + 1)).0.0" ;;
    minor) next="$major.$((minor + 1)).0" ;;
    patch) next="$major.$minor.$((patch + 1))" ;;
    *) echo "❌ Use major, minor, patch ou current."; exit 1 ;;
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
  changelog "$next"
  git add "$manifest" Cargo.lock "$changelog_file" "$changelog_dir"
  git commit -q -m "chore(release): v$next"
  git push -q -u "$remote" "release/v$next"
  gh pr create --base main --title "chore(release): v$next" \
    --body "Bump crustdb-app to $next and move \`changelog.d/\` into the CHANGELOG. After merge, run \`make release-tag\` to tag v$next and build the installers."
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
  changelog) changelog "${2:-}" ;;
  *) echo "Uso: $0 bump|tag|changelog X.Y.Z"; exit 1 ;;
esac
