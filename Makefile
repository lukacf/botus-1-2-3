CARGO := ./scripts/repo-cargo

.PHONY: check test lint fmt check-meta test-meta lint-meta check-mcp test-mcp lint-mcp test-mcp-host test-installed-mcp
check:
	$(CARGO) check --locked --all-targets
test:
	$(CARGO) test --locked --all-targets
lint:
	$(CARGO) clippy --locked --all-targets -- -D warnings
fmt:
	$(CARGO) fmt --all -- --check
check-meta:
	$(CARGO) check --locked --all-targets --features meta
test-meta:
	$(CARGO) test --locked --all-targets --features meta
lint-meta:
	$(CARGO) clippy --locked --all-targets --features meta -- -D warnings
check-mcp:
	$(CARGO) check --locked --all-targets --features mcp
test-mcp:
	$(CARGO) test --locked --all-targets --features mcp
lint-mcp:
	$(CARGO) clippy --locked --all-targets --features mcp -- -D warnings

# Explicit full host integration lane; the default engine lane stays small.
test-mcp-host:
	$(CARGO) test --locked --all-targets --features mcp-host-tests

# Install into a fresh temporary root and exercise the installed relay.
test-installed-mcp:
	./scripts/test-installed-mcp
