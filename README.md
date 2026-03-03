# treb

Deployment orchestration CLI for Foundry projects.

## Prerequisites

- **Rust nightly** — the project includes a `rust-toolchain.toml` that selects the nightly channel automatically.

## Build

```sh
cargo build --workspace
```

## Run

```sh
cargo run -p treb-cli -- --help
cargo run -p treb-cli -- --version
cargo run -p treb-cli -- run
```

## Test

```sh
cargo test --workspace
```

## Lint

```sh
cargo fmt --check
cargo clippy --workspace -- -D warnings
```

## Project Structure

```
crates/
  treb-cli/    # Binary crate — CLI entry point (clap)
  treb-core/   # Library crate — shared types, errors, foundry re-exports
```

## License

MIT OR Apache-2.0
