CARGO := ./scripts/repo-cargo

.PHONY: check test lint fmt check-mobkit test-mobkit lint-mobkit check-mcp test-mcp lint-mcp test-mcp-host
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
check-mcp:
	$(CARGO) check --locked --all-targets --features mcp
test-mcp:
	$(CARGO) test --locked --all-targets --features mcp
lint-mcp:
	$(CARGO) clippy --locked --all-targets --features mcp -- -D warnings

# Explicit full host integration lane; the default engine lane stays small.
test-mcp-host:
	$(CARGO) test --locked --all-targets --features mcp-host-tests
