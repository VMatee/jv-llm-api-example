# Linux setup

Install Git, Python 3.10+ and Rust/Cargo, plus your distribution's native linker
and build tools for Rust compilation.

```bash
git clone https://github.com/VMatee/jv-llm-api-example.git
cd jv-llm-api-example
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r python/requirements.txt
cargo build --manifest-path rust/Cargo.toml
python python/jv_responses_example.py "Summarize this document." --file examples/sample-document.txt
cargo run --manifest-path rust/Cargo.toml --bin jv-responses-example -- "Summarize this document." --attach file:examples/sample-document.txt
```

Both prompt for a password. Never bypass TLS verification or supply passwords
on the command line. See [mixed inputs and retry rules](../README.md).
