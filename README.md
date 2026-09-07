# JV LLM API examples

Reference clients for JV Server in Python, C, C++, and Rust. Every language now
has two explicit paths:

- `jv_responses_example` uses the structured, asynchronous `/v1/responses`
  pilot. Use it for new text and coding-agent integrations.
- `jv_api_example` keeps the established `/v1/jobs` API for attachments,
  owner-scoped conversation follow-ups, generated-file downloads, and existing
  integrations.

The Responses route is compatible with the documented JV pilot subset. It is
not a claim of complete OpenAI Responses API compatibility. Both APIs reuse the
same JV account, server-side provider assignment, queue, browser workers, and
canonical logging. Client tools run only on the client.

The default origin is `https://ai.openjvspace.com`. You need a JV username and
password. Passwords are read with a hidden prompt and are never accepted as a
command-line option.

## Choose a language

| Language | Structured example | Legacy jobs example | Guide |
|---|---|---|---|
| Python 3.10+ | `python/jv_responses_example.py` | `python/jv_api_example.py` | [Python](python/README.md) |
| C11 | `c/jv_responses_example.c` | `c/jv_api_example.c` | [C](c/README.md) |
| C++17 | `cpp/jv_responses_example.cpp` | `cpp/jv_api_example.cpp` | [C++](cpp/README.md) |
| Rust stable | `rust/src/bin/jv-responses-example.rs` | `rust/src/main.rs` | [Rust](rust/README.md) |

For installation from a clean computer, see [Linux](docs/linux.md),
[macOS](docs/macos.md), or [Windows](docs/windows.md). The structured wire
contract, polling, tool continuation, idempotency, and limitations are in the
[Responses API guide](docs/responses-api.md).

## Quick structured requests

Build or install the dependencies from the language guide, then run:

```bash
python ./python/jv_responses_example.py "Explain recursion in simple terms."
./c/build/jv_responses_example "Explain recursion in simple terms."
./cpp/build/jv_responses_example "Explain recursion in simple terms."
cargo run --manifest-path rust/Cargo.toml --bin jv-responses-example -- \
  "Explain recursion in simple terms."
```

Each command logs in, sends one idempotent structured request, polls the
long-running response, validates exactly one completed assistant text item,
prints it, and logs out. Add `--json` to inspect the complete safe response.

## Client-side tool round

The Python, C, C++, and Rust structured examples all support `--tool-demo`.
The request declares one strict, harmless `get_client_platform` function. JV
may return a validated function call; the example checks its name and empty
JSON arguments, runs the allowlisted function locally, and submits its result
with `function_call_output` for a second inference round.

```bash
python ./python/jv_responses_example.py \
  "Use the tool, then tell me which client platform is running." \
  --tool-demo
```

The server never runs this client tool. Real applications must keep their own
allowlist, sandbox, approval policy, and durable record of processed `call_id`
values before adding tools with side effects.

## Files and legacy conversations

The current Responses pilot is text-only. Keep using `/v1/jobs` when a request
needs an attachment, a legacy conversation ID, or generated-file download:

```bash
python ./python/jv_api_example.py \
  "Summarize the attached document." \
  --file ./examples/sample-document.txt

python ./python/jv_api_example.py \
  "Now list the three most important actions." \
  --conversation-id YOUR_CONVERSATION_ID
```

The equivalent `jv_api_example` binary is available for C, C++, and Rust. This
fallback remains supported; the new examples do not change it.

## Credentials and account isolation

The username defaults to `test`. Select another account with `--username` or
temporarily set `JV_API_USERNAME`. For controlled automation, load the password
from an approved secret manager into `JV_API_PASSWORD` only for the client
process and remove it afterward.

```bash
export JV_API_USERNAME="your-username"
read -rsp "JV LLM password: " JV_API_PASSWORD; echo
export JV_API_PASSWORD
python ./python/jv_responses_example.py "Return a concise status."
unset JV_API_PASSWORD
```

Never place a password or bearer token in source, command-line arguments,
URLs, `.env` files, Git history, or logs. Each bearer token can access only the
owning account's responses and jobs. Provider, model, effort, routing, and
fallback remain controlled by the authenticated user's server assignment.

## Repository layout

```text
jv-llm-api-example/
├── python/                 # structured and legacy Python clients
├── c/                      # structured and legacy C clients
├── cpp/                    # structured and legacy C++ clients
├── rust/                   # reusable async library, two CLIs, tests
├── docs/
│   ├── responses-api.md
│   ├── linux.md
│   ├── macos.md
│   └── windows.md
└── examples/sample-document.txt
```

## Retry rule

Polling GET requests are safe to repeat. A local timeout does not cancel the
server response or job. `/v1/responses` POSTs use an `Idempotency-Key`; retain
and reuse the same key only when explicitly retrying the same logical round.
Never generate a new key to retry an uncertain submission. The legacy
`/v1/jobs` POST has no client idempotency key and must not be retried
automatically after an ambiguous network result.
