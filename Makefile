# Envia todas as branches locais feat/* para o origin (nunca a main).
REMOTE ?= origin
BRANCHES := $(shell git for-each-ref --format='%(refname:short)' 'refs/heads/feat/*')

.PHONY: push-branches list-branches lockfix test release rc hooks lint

# Release: pergunta a versão (X.Y.Z ou X.Y.Z-rcN). Bump + PR se preciso; senão cria a release no GitHub,
# o que dispara .github/workflows/release.yml (instaladores). Rode de novo após o merge do PR de bump.
release:
	@./packaging/release.sh release

# Release candidate: pergunta nome e título; notas = mudanças na main desde a última RC; marca pre-release.
rc:
	@./packaging/release.sh rc

# Resolve conflito no Cargo.lock: parte de um lado (SIDE=ours|theirs) e o cargo
# reconcilia com os Cargo.toml mesclados, sem atualizar as demais versões.
SIDE ?= ours
lockfix:
	git checkout --$(SIDE) Cargo.lock
	cargo metadata --format-version 1 > /dev/null
	git add Cargo.lock

list-branches:
	@printf '%s\n' $(BRANCHES)

push-branches:
	@test -n "$(BRANCHES)" || { echo "nenhuma branch feat/*"; exit 1; }
	@for b in $(BRANCHES); do \
		echo ">> push $$b"; \
		git push -u $(REMOTE) "$$b" || exit 1; \
	done




# Ativa os hooks versionados em .githooks/ (fmt + clippy + testes no pre-commit).
hooks:
	git config core.hooksPath .githooks

lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings

test: lint
	cargo deny check
	cargo nextest run --workspace --fail-fast

