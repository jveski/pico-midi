.PHONY: build-2040 build-2350 check clippy fmt clean

# Default target
build: build-2040

build-2040:
	cargo build --release --features rp2040 --target thumbv6m-none-eabi
	elf2uf2-rs target/thumbv6m-none-eabi/release/midictrl midictrl-rp2040.uf2

build-2350:
	cargo build --release --no-default-features --features rp2350 --target thumbv8m.main-none-eabihf
	cp target/thumbv8m.main-none-eabihf/release/midictrl.uf2 midictrl-rp2350.uf2

check:
	cargo check --features rp2040 --target thumbv6m-none-eabi
	cargo check --no-default-features --features rp2350 --target thumbv8m.main-none-eabihf

clippy:
	cargo clippy --features rp2040 --target thumbv6m-none-eabi -- -D warnings
	cargo clippy --no-default-features --features rp2350 --target thumbv8m.main-none-eabihf -- -D warnings

fmt:
	cargo fmt --check

clean:
	cargo clean
