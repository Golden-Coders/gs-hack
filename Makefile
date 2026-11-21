# Demo: od świeżego klona w 2 komendach: `git clone` + `make demo`.
API_URL    ?= http://localhost:$(or $(API_PORT),8080)
INGEST_URL ?= http://localhost:8081
API_TOKEN  ?= dev-token
export API_URL INGEST_URL API_TOKEN

.PHONY: help demo demo-events down test

help: ## Lista celów
	@grep -E '^[a-z-]+:.*##' $(MAKEFILE_LIST) | sed 's/:.*##/\t/'

demo: ## Stack (build + up), czekanie na health, scenariusz demo
	docker compose up -d --build
	@echo "Czekam na api ($(API_URL)) i ingest ($(INGEST_URL)), max 180 s..."
	@for i in $$(seq 1 90); do \
	  if curl -sf $(API_URL)/health >/dev/null && [ "$$(curl -s -o /dev/null -w '%{http_code}' $(INGEST_URL)/health)" != "000" ]; then exit 0; fi; \
	  sleep 2; \
	done; echo "Timeout: stack nie wstał (docker compose logs)"; exit 1
	scripts/demo/run.sh

demo-events: ## Sam generator scenariusza (stack już działa)
	scripts/demo/run.sh

down: ## Zatrzymaj stack i usuń wolumeny
	docker compose down -v

test: ## Testy Rusta
	cargo test --workspace
