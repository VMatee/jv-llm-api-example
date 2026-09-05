# Rust JV LLM API example

Async reusable `jv_ai_client` library and separate `jv-api-example` CLI. Uses
the same public API as [the Python client](../python/jv_api_example.py):
username/password login, bearer authentication, multipart jobs, polling,
conversation follow-ups, authenticated response-file downloads, and logout.
Server-side account assignments control provider/model selection.

## Install and build

Install current stable Rust using [rustup](https://rust-lang.org/tools/install/).
Linux needs a C compiler/linker (for example `build-essential` on Ubuntu);
macOS needs Xcode Command Line Tools; Windows needs Visual Studio C++ Build
Tools and the Windows SDK. HTTP uses reqwest with rustls; libcurl and OpenSSL
are not required. Run all commands from the repository root:

```bash
cargo build --manifest-path rust/Cargo.toml --release
```

`Cargo.lock` is committed. Use `--locked` in automation for reproducible
resolution. The binary is `rust/target/release/jv-api-example` (with `.exe` on
Windows). `cargo run` also works as shown below.

## Login and ask a question

```bash
cargo run --manifest-path rust/Cargo.toml -- \
  "Explain recursion in simple terms" --username your-username
```

Each CLI invocation logs in, submits **one** job, waits for terminal status,
optionally downloads artifacts, and logs out. The password prompt is hidden;
there is no `--password` option. The username defaults to `test`, matching
Python. The default origin is `https://ai.openjvspace.com`.

The login body is exactly `username`, `password`, and `remember_me: false`.
Authenticated requests send `Authorization: Bearer <token>`. All requests
include `X-JV-CSRF: 1`. Tokens remain in memory, are not returned by `login()`,
and are never printed. The client uses no persistent cookie/session store.

## File uploads

```bash
cargo run --manifest-path rust/Cargo.toml -- \
  "Summarize this file" --file ./examples/sample-document.txt

cargo run --manifest-path rust/Cargo.toml -- \
  "Analyze these files" --file ./a.txt --file ./b.txt
```

The client streams regular, non-symlink files. All submissions use
`multipart/form-data`, including text-only requests: `text`, optional
`conversation_id`, and repeated `files` parts with filename and MIME type.
No transcript, routing override, or local attachment path is sent as a form
field. Default server limits are 10 files, 25 MiB per file, 100 MiB total, and
100 KiB of current text; the server enforces its configured limits.

## Continue a conversation

A first request omits `conversation_id`. Its job response and CLI stderr
report the conversation ID; JSON output also includes it.

```bash
cargo run --manifest-path rust/Cargo.toml -- \
  "Explain recursion in simple terms" --json > first.json

cargo run --manifest-path rust/Cargo.toml -- \
  "Continue from the previous answer" --conversation-id YOUR_CONVERSATION_ID
```

Use the ID from `first.json` for the second command. Submit only the **new**
text/files. The server reconstructs successful prior context. Follow-ups must
belong to the same account and wait until the prior turn is terminal. A
`result_ready: true` response can still be `running` during cleanup; the
client continues polling. Start an independent conversation by omitting the ID.

## JSON mode

```bash
cargo run --manifest-path rust/Cargo.toml -- "Give a short greeting" --json
```

stdout contains exactly one JSON object for a completed job:

```json
{
  "job_id": "opaque-job-id",
  "conversation_id": "opaque-conversation-id",
  "conversation_turn": 1,
  "status": "succeeded",
  "answer": "Hello!",
  "files": [],
  "downloaded_files": [],
  "error_code": null,
  "error_message": null
}
```

This is a CLI summary; the wire API uses `id` and `response.files`. Progress,
password prompts, and local errors go to stderr. Failed jobs also produce one
JSON object and exit nonzero. If login, submission, polling, or download fails
before a complete output is available, stdout is empty and stderr describes
the error. A logout failure preserves a completed JSON result but exits nonzero.
Unknown server fields and administrator routing are not copied into the summary.

## Download generated files

```bash
cargo run --manifest-path rust/Cargo.toml -- \
  "Create an image explaining recursion" --download-dir ./results --json
```

Artifacts are downloaded before logout. `files` contains API metadata and
`downloaded_files` contains local paths. No files is a valid result, including
for providers that do not produce downloadable artifacts.

Download checks match Python's contract and reject unsafe routes more strictly:

- Only relative `/v1/jobs/{this_job}/response-files/{artifact}` URLs are accepted.
  External URLs, encoded traversal, nested paths, queries, and fragments fail.
- Every download is authenticated. All redirects are disabled, including
  same-origin redirects, so credentials cannot follow a response elsewhere.
- Required `size_bytes` must be a positive integer: at most 25 MiB per file,
  100 MiB total, and 10 files. The whole manifest is checked before downloading.
- `Content-Length`, when present, must match metadata. Streaming byte counts
  must match exactly, even without Content-Length. Encoded responses fail.
- Names become safe portable single components; Windows reserved names are
  protected. Existing files and symlinks are never overwritten; collisions get
  numeric suffixes. Symlink destination directories are rejected.
- Private temporary files are removed on ordinary failure/cancellation and
  published with no-clobber semantics after validation. On Unix, new destination
  directories use mode 700 and downloaded files use mode 600. Already completed
  downloads remain if a later file fails. Use a destination you control.

## Environment variables and options

| Variable | Meaning |
|---|---|
| `JV_API_USERNAME` | Login username; `--username` takes precedence |
| `JV_API_PASSWORD` | Password for noninteractive execution; otherwise a hidden prompt |
| `JV_API_BASE_URL` | Approved API origin; `--base-url` takes precedence |

Bash example without putting the password in command history:

```bash
export JV_API_USERNAME="your-username"
read -rsp "JV LLM password: " JV_API_PASSWORD; echo
export JV_API_PASSWORD
cargo run --manifest-path rust/Cargo.toml -- "Give a short greeting" --json
unset JV_API_PASSWORD
```

Use your approved secret manager for unattended credentials. Never commit
credentials, put them in URLs/command arguments, or print them in logs. The
library does not read environment variables; pass configuration and credentials
explicitly. The CLI reads them. HTTP is accepted only for `localhost` and
`127.0.0.1` development; other origins require HTTPS with normal TLS verification.

```text
jv-api-example QUESTION [OPTIONS]
--base-url URL              API origin
--username USERNAME         Account username
--conversation-id ID        Owned conversation for a follow-up
--file PATH                 Current attachment; repeat as needed
--poll-interval SECONDS     Positive seconds between polls (default 2)
--wait-timeout SECONDS      Total local polling deadline (default 3600)
--json                     One terminal-result JSON object on stdout
--download-dir DIRECTORY    Download generated response files
```

## Reuse the library

Add a path dependency to another Rust project (adjust the path):

```toml
[dependencies]
jv-ai-client = { path = "../jv-llm-api-example/rust" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The following function accepts credentials from your application's secret input:

```rust
use jv_ai_client::{ClientConfig, JobStatus, JvClient, JvJobRequest, Result};

async fn ask(username: &str, password: &str) -> Result<()> {
    let mut client = JvClient::new(ClientConfig::default())?;
    client.login(username, password).await?;
    let operation: Result<()> = async {
        let created = client.submit_job(JvJobRequest::new("Explain recursion")).await?;
        let result = client.wait_for_job(&created.id).await?;
        if result.status == JobStatus::Succeeded {
            println!("{}", result.answer.as_deref().unwrap_or_default());
            client.download_response_files(&result, "./results").await?;
            let followup = JvJobRequest {
                text: "Show a small example".into(),
                conversation_id: Some(result.conversation_id),
                files: vec![],
            };
            let next = client.submit_job(followup).await?;
            let _completed = client.wait_for_job(&next.id).await?;
        }
        Ok(())
    }.await;
    // Always attempt logout, including when submission/poll/download failed.
    let logout = client.logout().await;
    operation?;
    logout
}
```

`get_job(id)` performs one GET. `wait_for_job(id)` adds retries and deadline
handling; `wait_for_job_with_progress(id, callback)` also reports progress.
`wait_for_job` returns both succeeded and failed jobs: inspect `JobStatus`.
`ClientConfig` controls request timeout (120 seconds by default), polling,
total wait, and consecutive transient-error budget (8 by default).

`logout()` attempts server revocation and always drops the local token, even
on failure. Rust Drop cannot perform async revocation; callers must await
logout on their exit paths. The CLI attempts logout after job/download errors
and Ctrl-C; a forcibly killed process cannot guarantee revocation.

## Errors, timeouts, and retry safety

Public `Error` variants are typed and redact raw transport errors/response bodies.
HTTP errors include status and parsed `Retry-After` (seconds or HTTP date).

- HTTP 409 can indicate a busy conversation/upload admission. HTTP 429 means
  throttling. Inspect the state and respect Retry-After before a new action.
- **POST /v1/jobs is never automatically retried**, including on 409/429.
  Network errors, timeouts, 5xx, redirects, or malformed 202 responses produce
  `SubmissionUncertain`: the job may already exist. Reconcile via account
  history before deciding whether another submission is safe.
- Safe polling GETs retry network errors, malformed responses, 408/409/429,
  and 5xx with bounded exponential backoff (up to 30 seconds), at least the
  server's Retry-After, and a finite consecutive-error budget. 401/403/404 fail
  immediately. Successful GETs reset the consecutive-error budget.
- The total polling deadline includes in-flight HTTP requests. If Retry-After
  exceeds it, waiting expires without another early request. `WaitTimeout`
  includes the job ID. The server job continues; log in and use `get_job` or
  `wait_for_job` with that same ID to resume. Never resubmit to resume polling.
- Current nonterminal statuses are queued, dispatching, waiting_for_provider,
  running, and waiting_for_auth. Only succeeded/failed are terminal. Unknown
  statuses are malformed data, never implicit success. Server failure fields
  are available on `Job` and in JSON output.
- Exit codes: 0 success, 1 operation/job/logout failure, 2 CLI usage error,
  130 Ctrl-C. Diagnostics never include login responses or bearer tokens.

## Tests and CI

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml
cargo build --manifest-path rust/Cargo.toml --release
```

The Rust GitHub Actions workflow runs all four checks. Tests use loopback mock
HTTP servers and fake credentials; they never start a browser worker or submit
live jobs. Coverage includes authentication headers/body, multipart with and
without files, two-turn conversation handling, nonterminal result_ready,
409/429/Retry-After, uncertain POST no-retry, timeout, malformed responses,
manifest/URL/filename limits, symlinks, collisions, download errors, logout on
failure, and CLI JSON/exit codes.

Optional live smoke test (requires a real assigned account; **creates two real
jobs**). Supply credentials through the variables above, then explicitly run:

```bash
cargo test --manifest-path rust/Cargo.toml --test live -- --ignored
```

By default it uploads a harmless temporary text file, waits for success, continues
the same conversation, downloads any returned artifacts, and logs out. To require
actual response-file output, use an artifact-capable assignment and set:

```bash
export JV_API_LIVE_PROMPT="Read the attached document and create a downloadable text file containing its reference word."
export JV_API_LIVE_REQUIRE_FILES=1
cargo test --manifest-path rust/Cargo.toml --test live -- --ignored
unset JV_API_LIVE_PROMPT JV_API_LIVE_REQUIRE_FILES
```

The strict variant fails if no artifact is returned. Successful mock tests do not
claim live-provider verification; live tests require independent credentials.
