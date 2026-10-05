CARGO := ./scripts/repo-cargo

.PHONY: check test lint fmt
check:
	$(CARGO) check --all-targets
test:
	$(CARGO) test --all-targets
lint:
	$(CARGO) clippy --all-targets -- -D warnings
fmt:
	$(CARGO) fmt --all -- --check

