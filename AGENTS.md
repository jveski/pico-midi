# pico-midi

Embedded MIDI controller firmware for Raspberry Pi Pico (RP2040 and RP2350) written in Rust using the Embassy async framework, with a vanilla HTML/JS web configurator UI.

## Project Structure

- `firmware/` - Rust embedded firmware (`#![no_std]`, Embassy, USB MIDI + CDC-ACM serial)
  - `src/main.rs` - Entry point, USB setup, MIDI polling loop, serial handler
  - `src/config.rs` - Configuration data structures (serde-derived)
  - `src/expr.rs` - Expression evaluation engine (bytecode compiler and VM)
  - `src/input.rs` - Input drivers (buttons, touch pads, pots, LDR, accelerometer via I2C)
  - `src/input_state.rs` - Shared atomic input state for cross-task access
  - `src/serial.rs` - Serial protocol with COBS framing, flash config load/save via postcard
  - `.cargo/config.toml` - Cargo build config (ARM target flags, defmt, elf2uf2-rs runner)
  - `memory-rp2040.x` / `memory-rp2350.x` - Linker scripts for each chip
  - `build.rs` - Build script selecting memory layout based on feature flag
- `ui/` - Web-based configurator (plain HTML/CSS/JS, native Web Components, no build step)
  - `components/` - One Web Component per .js file (app, config-panel, connect-banner, expr, item-list, midi-channel, pinout-modal, protocol, save-banner, etc.)
  - `configurator.html` - Main configurator page (Web Serial connection to device)
  - `index.html` - Landing/docs page with firmware download links
  - `style.css` - Shared styles (brutalist white-on-black aesthetic)
- `.github/workflows/` - CI: firmware clippy/fmt check, UF2 binary release builds, UI deploy to GitHub Pages

## Build and Verify

All firmware commands run from the `firmware/` directory using the nightly Rust toolchain.

### Check both targets (must pass before committing)

```sh
cargo clippy --features rp2040 --target thumbv6m-none-eabi -- -D warnings
cargo clippy --no-default-features --features rp2350 --target thumbv8m.main-none-eabihf -- -D warnings
cargo fmt --check
```

### Build firmware

```sh
cargo build --release                                                    # RP2040 (default)
cargo build --release --no-default-features --features rp2350 --target thumbv8m.main-none-eabihf  # RP2350
```

### UI

No build step. Files are served as-is and deployed to GitHub Pages. No tests.

## Conventions

- Rust: `#![no_std]`, Embassy async patterns, all input state shared via atomics in `input_state.rs`
- Two chip targets selected via Cargo features: `rp2040` (default, thumbv6m-none-eabi) and `rp2350` (thumbv8m.main-none-eabihf)
- Serial protocol: serde + postcard + COBS framing between firmware and UI
- Config persistence: stored in flash via postcard serialization
- UI: native Web Components, one per file in `ui/components/`, no frameworks or build tools
- Expressions: small language compiled to bytecode in browser, executed on microcontroller VM
- CI runs on nightly Rust with both targets - all clippy warnings are errors (`-D warnings`)

## Rules

- ALWAYS run clippy for BOTH targets after making any Rust changes - use the exact commands from "Check both targets" above
- ALWAYS run `cargo fmt --check` after Rust changes and fix any formatting issues
- When making firmware changes, always consider both RP2040 and RP2350 - they have different memory layouts, GPIO behavior (especially touch pads), and target architectures
- The UI has no tests - verify changes by careful code review
- When working on a multi-step task, continue to completion without pausing for confirmation unless genuinely uncertain about direction
- Complete the full workflow: implement, verify with clippy/fmt, review non-trivial changes with a subagent, then commit
- Commit messages should be concise and descriptive, focusing on the "why"
