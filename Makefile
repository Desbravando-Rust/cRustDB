# Envia todas as branches locais feat/* para o origin (nunca a main).
REMOTE ?= origin
BRANCHES := $(shell git for-each-ref --format='%(refname:short)' 'refs/heads/feat/*')

.PHONY: push-branches list-branches

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