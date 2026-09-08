# Windows setup

Install Git, Python 3.10+ and Rust/Cargo. Follow Rust's installer instructions
for the native MSVC linker/build prerequisites. These are compiler dependencies,
not maintained C/C++ clients. In PowerShell:

```powershell
git clone https://github.com/VMatee/jv-llm-api-example.git
Set-Location jv-llm-api-example
py -3 -m venv .venv
.\.venv\Scripts\python.exe -m pip install -r python/requirements.txt
cargo build --manifest-path rust/Cargo.toml
.\.venv\Scripts\python.exe python/jv_responses_example.py "Summarize this document." --file examples/sample-document.txt
cargo run --manifest-path rust/Cargo.toml --bin jv-responses-example -- "Summarize this document." --attach file:examples/sample-document.txt
```

Both prompt for a password. Quote `"file:C:\My Documents\report.pdf"` for
Rust paths containing spaces. Never bypass TLS verification. See
[mixed inputs and retry rules](../README.md).
