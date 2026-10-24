#!/usr/bin/env bash
# Release (nunca faz push na main). Ponto de entrada: `make release`.
#   packaging/release.sh release       -> pergunta a versão (X.Y.Z ou X.Y.Z-rcN):
#                                         RC: cria a pre-release vX.Y.Z-rcN (não mexe em Cargo/CHANGELOG);
#                                         real com Cargo diferente (ou sem seção no CHANGELOG): bump + PR;
#                                         real com Cargo e CHANGELOG já na main: cria a release vX.Y.Z.
#                                         Criar a release dispara .github/workflows/release.yml (instaladores).
#   packaging/release.sh kind TAG CARGO_VERSION -> imprime rc|real|mismatch (usado pelo workflow).
#   packaging/release.sh notes X.Y.Z   -> imprime a seção `## [X.Y.Z]` do CHANGELOG.
#   packaging/release.sh changelog X.Y.Z -> converte os fragmentos em `## [X.Y.Z]` (usado pelo bump e pelo workflow).
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
  while read -r f; do
    [[ "$f" =~ ^[0-9]+\.(added|changed|fixed)\.md$ ]] || { echo "❌ Fragmento inválido: $changelog_dir/$f"; exit 1; }
  done < <(find "$changelog_dir" -mindepth 1 -maxdepth 1 ! -name .gitkeep -printf '%f\n')
  compgen -G "$changelog_dir/*.md" >/dev/null || { echo "❌ Nenhum fragmento em $changelog_dir/."; exit 1; }
}

check_section() {
  ! grep -qxF "## [$1]" "$changelog_file" || { echo "❌ $changelog_file já tem a seção [$1]."; exit 1; }
}

# Insere `## [X.Y.Z]` acima da versão anterior (abaixo do cabeçalho) e apaga os fragmentos.
changelog() {
  local version="${1:-}" body kind f n tmp
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "❌ Versão inválida (RC não gera seção): '$version'"; exit 1; }
  check_fragments
  check_section "$version"

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

  local cur major minor patch kind next="${1:-}"
  cur="$(current_version)"
  [[ "$cur" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || { echo "❌ Versão inválida em $manifest: '$cur'"; exit 1; }
  major="${BASH_REMATCH[1]}" minor="${BASH_REMATCH[2]}" patch="${BASH_REMATCH[3]}"

  echo "Versão atual: $cur"
  check_fragments
  if [ -z "$next" ]; then
    read -rp "Tipo de bump (major/minor/patch/current): " kind
    case "$kind" in
      current) next="$cur" ;;
      major) next="$((major + 1)).0.0" ;;
      minor) next="$major.$((minor + 1)).0" ;;
      patch) next="$major.$minor.$((patch + 1))" ;;
      *) echo "❌ Use major, minor, patch ou current."; exit 1 ;;
    esac
  fi

  if git ls-remote --exit-code --tags "$remote" "refs/tags/v$next" >/dev/null; then
    echo "❌ Tag v$next já existe em $remote."
    exit 1
  fi
  check_section "$next"
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
    --body "Bump crustdb-app to $next and move \`changelog.d/\` into the CHANGELOG. After merge, run \`make release\` again to publish v$next and build the installers."
}

# rc: -rcN (não checa o Cargo) | real: bate com o Cargo | mismatch: versão real diferente do Cargo.
kind() {
  local tag="${1:-}" cargo="${2:-}"
  if [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+-rc[0-9]+$ ]]; then echo rc
  elif [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    [ "$tag" = "v$cargo" ] && echo real || echo mismatch
  else
    echo "❌ Tag inválida: '$tag'" >&2
    exit 1
  fi
}

notes() { awk -v h="## [$1]" '$0 == h {f = 1; next} f && /^## / {exit} f' "$changelog_file"; }

release() {
  local version cargo tmp
  read -rp "Versão a liberar (X.Y.Z ou X.Y.Z-rcN): " version
  kind "v$version" 0 >/dev/null
  git fetch -q "$remote" main
  if git ls-remote --exit-code --tags "$remote" "refs/tags/v$version" >/dev/null; then
    echo "❌ Tag v$version já existe em $remote."
    exit 1
  fi
  if [[ "$version" == *-rc* ]]; then
    confirm "Criar a pre-release v$version a partir de $remote/main?"
    gh release create "v$version" --prerelease --target main --title "v$version" --notes "Release candidate $version."
  else
    cargo="$(git show "$remote/main:$manifest" | awk -F'"' '/^version = / {print $2; exit}')"
    if [ "$cargo" = "$version" ] && git show "$remote/main:$changelog_file" | grep -qxF "## [$version]"; then
      confirm "Criar a release v$version a partir de $remote/main?"
      tmp="$(mktemp)"
      git show "$remote/main:$changelog_file" >"$tmp"
      changelog_file="$tmp" notes "$version" >"$tmp.notes"
      gh release create "v$version" --target main --title "v$version" --notes-file "$tmp.notes"
      rm -f "$tmp" "$tmp.notes"
    else
      bump "$version"
      return
    fi
  fi
  echo "✅ Release v$version criada. Acompanhe os instaladores em: gh run list --workflow release.yml"
}

case "${1:-}" in
  release) release ;;
  bump) bump ;;
  kind) kind "${2:-}" "${3:-}" ;;
  notes) notes "${2:-}" ;;
  changelog) changelog "${2:-}" ;;
  *) echo "Uso: $0 release|bump|kind TAG CARGO|notes X.Y.Z|changelog X.Y.Z"; exit 1 ;;
esac
