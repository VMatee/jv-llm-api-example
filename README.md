# JV LLM API examples

Python and Rust examples for connecting your application to JV: send prompts, attach documents or images, check results, and continue a conversation.

## Choose your client

| Language | Get started | Requirements |
| --- | --- | --- |
| Python | [Python guide](python/README.md) | Python 3.10+ |
| Rust | [Rust guide](rust/README.md) | Rust/Cargo with edition 2024 support |

Setup guides: [Linux](docs/linux.md) · [macOS](docs/macos.md) · [Windows](docs/windows.md).

You need an existing JV account and internet access. The default service address is `https://ai.openjvspace.com`. Account capabilities are managed by your service administrator.

## Quick start with Python

```bash
git clone https://github.com/VMatee/jv-llm-api-example.git
cd jv-llm-api-example
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r python/requirements.txt
python python/jv_responses_example.py "Explain recursion." --username your-user
```

On Windows, use `.venv\Scripts\Activate.ps1` to activate the environment. The client asks for your password using hidden input. The illustrative default username `test` does not create an account.

## Send a document or image

```bash
python python/jv_responses_example.py "Summarize this document." --file examples/sample-document.txt
python python/jv_responses_example.py "Compare these attachments." --image screenshot.png --file report.pdf
```

For Rust:

```bash
cargo run --manifest-path rust/Cargo.toml --bin jv-responses-example -- "Summarize this document." --attach file:examples/sample-document.txt
```

Repeat attachment options to include multiple inputs; their order is preserved. PNG, JPEG, WebP, text, Markdown, PDF, JSON, CSV, Python source, and selected Office files are supported subject to account capabilities and validation limits. See the [API reference](docs/responses-api.md) for exact limits.

## Available workflows

- **Responses:** asynchronous text and attachment requests, status polling, and client-executed tool continuation.
- **Jobs:** direct requests, follow-up conversations, and generated-file downloads using `python/jv_api_example.py` or the Rust `jv-api-example` command.
- **Tool demonstration:** add `--tool-demo` to run a fixed local platform-information function. Always validate and explicitly allow client-side tool actions in your own application.

These examples implement the documented JV API features. Do not assume compatibility with every feature in other SDKs or services.

## Credentials, retries, and results

Use hidden password input. Approved automation may obtain `JV_API_PASSWORD` from a protected secret manager. Never put passwords in command-line arguments or commit credentials, tokens, `.env` files, or private request content.

For Responses requests, retain `--idempotency-key YOUR-STABLE-KEY` and the exact original input when reconciling an uncertain submission. Starting a new key may create duplicate work. Polling timeouts do not cancel remote work. File staging expires after two hours; see the API reference before implementing durable workflows.

Review results before relying on them. Service availability and supported capabilities depend on your account. Follow the [verification guide](docs/verification.md) to run local checks.
