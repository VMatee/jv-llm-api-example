# JV LLM API — Python and Rust

Maintained reference clients for JV Server's asynchronous `/v1/responses` and
established `/v1/jobs` APIs. C/C++ examples were retired and remain in Git history.

Structured input supports text, images, staged files, ordered mixed content,
JSON function tools, image-bearing `view_image` result continuation, and the
custom/freeform `apply_patch` flow certified for pinned Codex `0.149.1`.
This is a documented JV subset, **not a drop-in OpenAI Responses implementation
or complete unmodified Codex compatibility**. JV CLI integration remains separate.

## Start

See [Python](python/README.md), [Rust](rust/README.md), and platform setup for
[Linux](docs/linux.md), [macOS](docs/macos.md), [Windows](docs/windows.md).
Run from this repository root:

```bash
python python/jv_responses_example.py "Summarize this document." --file examples/sample-document.txt
cargo run --manifest-path rust/Cargo.toml --bin jv-responses-example -- "Summarize this document." --attach file:examples/sample-document.txt
```

The origin defaults to `https://ai.openjvspace.com`. Choose your account with
`--username`; the illustrative default `test` does not create an account.
Passwords use a hidden prompt, never an argument. Approved automation may
supply `JV_API_PASSWORD` through a secret manager. Never commit credentials,
tokens, local notes, `.env` files or production request content.

## Mixed attachments and tools

```bash
python python/jv_responses_example.py "Compare these attachments." --image screenshot.png --file report.pdf --tool-demo
cargo run --manifest-path rust/Cargo.toml --bin jv-responses-example -- "Compare these attachments." --attach image:screenshot.png --attach file:report.pdf --tool-demo
```

Repeat attachment options; their order is preserved. Both clients support
`--image-detail auto|high`. Empty quoted text with attachments creates
attachment-only input. `--tool-demo` runs only a fixed local platform function;
the server never executes tools or uploaded source. Tool continuation retains
the original attachments without requiring the client to resend them.

The Python and Rust libraries also include protocol-only constructors and tests
for image-bearing function results and pinned `apply_patch` custom calls/results.
They do not execute patches. See the exact certified shapes and limits in the
[Responses contract](docs/responses-api.md).

Certified input: PNG/JPEG/WebP; TXT, Markdown, PDF, JSON, CSV, Python source,
and conservative DOCX/XLSX/PPTX. Server validation, not extensions alone,
determines admission. Unsupported provider assignments fail without rerouting.
See [exact contract, limits and capability boundaries](docs/responses-api.md).

Legacy `python/jv_api_example.py` and Rust `jv-api-example` remain available for
`/v1/jobs`, legacy conversations and generated-file downloads.

## Retry and logging

Use `--idempotency-key YOUR-STABLE-KEY` and retain the exact input. Upload and
continuation keys derive from that key. Never retry uncertain work with a new
key. Libraries should persist staged IDs, each request and each round's key
privately; CLI examples are not durable workflow journals. In particular,
staging expires after two hours. POSTs are not automatically retried. Polling
timeout does not cancel server work.

Accepted server requests/results have private, owner-scoped canonical records.
Attachment bytes have bounded retention, not ordinary-log/base64 dumps.
Rejected requests do not necessarily create an inference history entry.
No production logs or credentials belong in this repository.

## Tests

```bash
python -m unittest discover -s python -p 'test_*.py'
cargo test --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
```

Normal tests use local fixtures/mock servers. Live tests are opt-in. Server
production acceptance used a bounded internal Rust probe; public clients have
their own offline tests.
