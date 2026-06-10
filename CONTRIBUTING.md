# Contributing

This public repository uses MIT licensing and Rust stable.

Before opening a pull request, run:

```sh
cargo fmt --check
cargo clippy --all-targets
cargo test
```

Do not commit local path overrides for sibling UbU crates. Use git-revision
dependencies in `Cargo.toml` and local `.cargo/config.toml` patches during
development.
