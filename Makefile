CARGO := ./scripts/repo-cargo

.PHONY: check test lint fmt check-mobkit test-mobkit lint-mobkit
check:
	$(CARGO) check --locked --all-targets
test:
	$(CARGO) test --locked --all-targets
lint:
	$(CARGO) clippy --locked --all-targets -- -D warnings
fmt:
	$(CARGO) fmt --all -- --check
check-mobkit:
	$(CARGO) check --locked --all-targets --features mobkit
test-mobkit:
	$(CARGO) test --locked --all-targets --features mobkit
lint-mobkit:
	$(CARGO) clippy --locked --all-targets --features mobkit -- -D warnings
