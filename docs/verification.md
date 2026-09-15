# Verify the examples

Run from the repository root after installing the dependencies:

```bash
python -m unittest discover -s python -p 'test_*.py'
python -m compileall -q python
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo test --manifest-path rust/Cargo.toml --locked
cargo clippy --manifest-path rust/Cargo.toml --locked --all-targets --all-features -- -D warnings
```

Use a Rust toolchain that supports edition 2024, with the formatting and linting components installed. The normal tests use local fixtures and mock HTTP services. Live integration tests are opt-in and can consume account quota.

Before integrating the examples into your application, verify authentication, supported file types and limits, status handling, idempotent retries, and download validation for your own account. Keep test credentials synthetic and production data out of the repository.

Passing automated tests does not guarantee live availability or every account capability. These are reference examples; durable applications need their own secure storage, retry policy, and operational monitoring.
