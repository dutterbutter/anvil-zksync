# Fetch submodule system contracts
fetch-contracts:
	git submodule update --init --recursive

# Build the system contracts
build-contracts:
	./scripts/refresh_contracts.sh
	./scripts/refresh_l1_sidecar_contracts.sh
	./scripts/refresh_test_contracts.sh
	./scripts/refresh_e2e_contracts.sh

# Clean the system contracts
clean-contracts:
	cd contracts/system-contracts && yarn clean
	rm -rf src/deps/contracts

# Build the Rust project
rust-build:
	cargo build --release

# Run local after building everything
run: all
	./target/release/anvil-zksync run

# Build the Rust documentation
rust-doc:
	cargo doc --no-deps --open

# Serve docs site locally
docs-site:
	cd docs/book && mdbook serve --open

# Lint checks
lint:
	cd e2e-tests && yarn && yarn lint && yarn fmt && yarn typecheck
	cd e2e-tests-rust && cargo fmt --all -- --check
	cd spec-tests && cargo fmt --all -- --check
	cargo fmt --all -- --check
	cargo clippy --tests -p anvil-zksync -- -D warnings --allow clippy::unwrap_used
	cd e2e-tests-rust && cargo clippy --tests -- -D warnings --allow clippy::unwrap_used
	cd spec-tests && cargo clippy --tests -- -D warnings --allow clippy::unwrap_used

# Fix lint errors
lint-fix:
	cd e2e-tests && yarn && yarn lint:fix && yarn fmt:fix
	cargo clippy --fix
	cargo fmt
	cd e2e-tests-rust && cargo fmt --all
	cd e2e-tests-rust && cargo clippy --fix
	cd spec-tests && cargo fmt --all
	cd spec-tests && cargo clippy --fix

# Run unit tests for Rust code
test:
	cargo test

# Run e2e tests against running anvil-zksync
test-e2e:
	./scripts/execute-e2e-tests.sh

# Build everything
all: fetch-contracts build-contracts rust-build

# Clean everything
clean: clean-contracts

# Create new draft release based on Cargo.toml version
new-release-tag:
	@VERSION_NUMBER=$$(grep '^version =' Cargo.toml | awk -F '"' '{print $$2}') && \
	git tag -a v$$VERSION_NUMBER -m "Release v$$VERSION_NUMBER" && \
	echo "\n\033[0;32mGit tag creation SUCCESSFUL! Use the following command to push the tag:\033[0m" && \
	echo "git push origin v$$VERSION_NUMBER"

# Create the rust book
book:
	mdbook build docs/rustbook

.PHONY: build-contracts clean-contracts rust-build lint test test-e2e all clean build-% new-release-tag book
