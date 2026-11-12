
fmt:
	cargo +nightly fmt --all

check:
    cargo +nightly check --all

clippy:
	cargo clippy --all --all-targets --all-features -- \
		-D warnings -D clippy::enum_glob_use

test:
	CARGO_BUILD_TARGET_DIR="target/test" cargo test --all -- --show-output

build-dev:
	cargo build --release

ci: fmt check clippy test

run:
    cargo watch -c -x "run --release"

# start-dev:
# 	cd ops/docker; docker compose --profile dev up -d
