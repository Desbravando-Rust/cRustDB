# Envia todas as branches locais feat/* para o origin (nunca a main).
REMOTE ?= origin
BRANCHES := $(shell git for-each-ref --format='%(refname:short)' 'refs/heads/feat/*')

.PHONY: push-branches list-branches lockfix icons test release release-tag

# Release: 1) bump de versão + PR; 2) após o merge, tag vX.Y.Z que dispara os instaladores no CI.
release:
	@./packaging/release.sh bump

release-tag:
	@./packaging/release.sh tag

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


icons: 
	cargo run -p icons_generator --release


ci:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo deny check
	cargo nextest run --workspace --fail-fast