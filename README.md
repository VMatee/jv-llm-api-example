# JV LLM API examples

Small Python, C++, C, and Rust clients for asking questions through the JV LLM API.
Every example follows the same safe login, submission, polling, and logout
contract. Provider, model, reasoning mode, and fallback remain controlled by
the user's server-side assignment.

The example connects to `https://ai.openjvspace.com` by default. You need a JV
LLM username and password before using it.

## Choose a language

| Language | Minimum | HTTP/JSON libraries | Guide |
|---|---|---|---|
| Python | Python 3.10 | requests | [Python guide](python/README.md) |
| C++ | C++17 | libcurl, nlohmann-json | [C++ guide](cpp/README.md) |
| C | C11 | libcurl, json-c | [C guide](c/README.md) |
| Rust | Current stable Rust | tokio, reqwest, serde | [Rust guide](rust/README.md) |

All languages require a JV LLM account and Internet access to
`https://ai.openjvspace.com`.

## Choose your operating system

For a complete setup from a clean computer, use the guide matching your
system:

- [Linux setup and examples](docs/linux.md)
- [macOS setup and examples](docs/macos.md)
- [Windows setup and examples](docs/windows.md)

Each operating-system guide installs Python, C++, and C dependencies and then
shows both text-only and file-attachment requests. For Rust installation and
usage on these systems, see the [Rust guide](rust/README.md).

## Repository layout

```text
jv-llm-api-example/
├── README.md
├── python/
│   ├── __init__.py
│   ├── README.md
│   ├── jv_api_example.py
│   └── requirements.txt
├── cpp/
│   ├── README.md
│   ├── CMakeLists.txt
│   └── jv_api_example.cpp
├── c/
│   ├── README.md
│   ├── CMakeLists.txt
│   └── jv_api_example.c
├── rust/
│   ├── Cargo.toml
│   ├── Cargo.lock
│   ├── README.md
│   ├── src/          # reusable library and separate CLI
│   └── tests/        # offline tests and opt-in live smoke test
├── docs/
│   ├── linux.md
│   ├── macos.md
│   └── windows.md
└── examples/
    └── sample-document.txt
```

Run the documented commands from the repository root. This keeps language
implementations isolated while sharing the platform guides and harmless sample
attachment.

## Installation

Clone this repository and enter its directory:

```bash
git clone https://github.com/VMatee/jv-llm-api-example.git
cd jv-llm-api-example
```

For Python, create a virtual environment and install the dependency:

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r python/requirements.txt
```

For C++, see [cpp/README.md](cpp/README.md):

```bash
sudo apt-get install cmake g++ libcurl4-openssl-dev nlohmann-json3-dev
cmake -S cpp -B cpp/build -DCMAKE_BUILD_TYPE=Release
cmake --build cpp/build --parallel
./cpp/build/jv_api_example "What are the three primary colors?"
```

For C, see [c/README.md](c/README.md):

```bash
sudo apt-get install cmake gcc libcurl4-openssl-dev libjson-c-dev
cmake -S c -B c/build -DCMAKE_BUILD_TYPE=Release
cmake --build c/build --parallel
./c/build/jv_api_example "What are the three primary colors?"
```

On Windows PowerShell, activate the environment with:

```powershell
.venv\Scripts\Activate.ps1
python -m pip install -r python/requirements.txt
```

## Quick examples: with and without attachments

Each language uses one client for both request types. Omit `--file` for a
text-only request. Add `--file PATH` when the model should receive a document.
The repository includes `examples/sample-document.txt`, so every attachment
example below is ready to copy and run.

### Python

Without an attachment:

```bash
python ./python/jv_api_example.py "Explain recursion in simple terms."
```

With an attachment:

```bash
python ./python/jv_api_example.py \
  "Summarize the attached document." \
  --file ./examples/sample-document.txt
```

### C++

Without an attachment:

```bash
./cpp/build/jv_api_example "Explain recursion in simple terms."
```

With an attachment:

```bash
./cpp/build/jv_api_example \
  "Summarize the attached document." \
  --file ./examples/sample-document.txt
```

### C

Without an attachment:

```bash
./c/build/jv_api_example "Explain recursion in simple terms."
```

With an attachment:

```bash
./c/build/jv_api_example \
  "Summarize the attached document." \
  --file ./examples/sample-document.txt
```

### Rust

Build and run (see the [Rust guide](rust/README.md) for the reusable async
library, conversation follow-ups, JSON output, and secure downloads):

```bash
cargo build --manifest-path rust/Cargo.toml --release
cargo run --manifest-path rust/Cargo.toml -- "Explain recursion in simple terms."
cargo run --manifest-path rust/Cargo.toml -- \
  "Summarize the attached document." --file ./examples/sample-document.txt
```

In every case, the first argument is the question. `--file` is optional and
may be repeated to attach several files. The client signs in, submits exactly
one job, polls that job, prints the result, and signs out.

## Account selection and password

The remainder of this page is the Python quick reference. The dedicated
[Python guide](python/README.md) collects its installation and usage examples
in one place. The default username is `test`. The program asks for the
password without displaying it. Use `--username` if your account has a
different username:

```bash
python ./python/jv_api_example.py \
  "Explain recursion in simple terms." \
  --username your-username
```

The client logs in, creates a job, displays status changes, prints the final
answer, and logs out.

## Use your own attachments

Use `--file` with an absolute or relative file path:

```bash
python ./python/jv_api_example.py \
  "Summarize this document." \
  --file ./documents/report.pdf
```

Repeat `--file` to send multiple files:

```bash
python ./python/jv_api_example.py \
  "Compare these two reports." \
  --file ./documents/report-one.pdf \
  --file ./documents/report-two.pdf
```

Default service limits are 10 files, 25 MiB per file, 100 MiB in total, and
100 KiB of question text. A deployment may configure smaller limits.

## Continue a conversation

Every new question prints a conversation ID. Pass that value to a later call
with `--conversation-id`:

```bash
python ./python/jv_api_example.py \
  "Now list the three most important actions." \
  --conversation-id YOUR_CONVERSATION_ID
```

Send only the new question and any new files. The service supplies the earlier
successful conversation context.

Do not reuse a conversation ID belonging to another account. Do not submit a
new follow-up while the previous turn in that conversation is still running.

## Download generated files

Use `--download-dir` when a response may include an image or file:

```bash
python ./python/jv_api_example.py \
  "Create an image explaining a Gaussian process." \
  --download-dir ./results
```

The client creates the destination directory when needed, validates the
returned file metadata, and downloads each file with a safe local name.

## Print the complete job result

Use `--json` to print the complete public job response instead of only the
answer:

```bash
python ./python/jv_api_example.py \
  "Return a short project status." \
  --json
```

Useful result fields include:

- `id`: the job ID;
- `conversation_id`: the conversation ID for follow-ups;
- `conversation_turn`: the turn number;
- `status` and `phase`: current progress;
- `queue_position`: the job's queue position when available;
- `answer`: the final answer after success;
- `response.files`: generated files available for authenticated download;
- `error_code` and `error_message`: safe failure details.

## Non-interactive use

For automation, provide the username and password through temporary environment
variables:

```bash
export JV_API_USERNAME="your-username"
read -rsp "JV LLM password: " JV_API_PASSWORD
export JV_API_PASSWORD

python ./python/jv_api_example.py "Return a concise status summary."

unset JV_API_PASSWORD
```

You can also set a different API origin when using an approved deployment:

```bash
export JV_API_BASE_URL="https://your-approved-api.example"
```

The client accepts HTTPS origins. Plain HTTP is accepted only for loopback
development addresses such as `127.0.0.1` and `localhost`.

Never put passwords or access tokens in source code, command-line arguments,
URLs, committed files, `.env` files, or application logs. For unattended use,
load the password from an approved secret manager and remove it from the
environment after the process exits.

## Command-line options

```text
python ./python/jv_api_example.py QUESTION [options]

--file PATH                 Attach a file; repeat for multiple files
--conversation-id ID        Continue an owned conversation
--base-url URL              Override the API origin
--username USERNAME         Override the default username
--poll-interval SECONDS     Time between status checks; default: 2
--wait-timeout SECONDS      Local polling timeout; default: 3600
--json                      Print the complete public job JSON
--download-dir DIRECTORY    Download generated response files
```

Run `python ./python/jv_api_example.py --help` to see the current option
descriptions.

## Use the client from Python

The `JVAIClient` class can be imported into another program:

```python
import getpass
from pathlib import Path

from python.jv_api_example import JVAIClient

client = JVAIClient("https://ai.openjvspace.com")
try:
    client.login("test", getpass.getpass("Password: "))
    created = client.submit_job(
        "Summarize the attached report.",
        file_paths=[Path("./documents/report.pdf")],
    )
    completed = client.wait_for_job(created["id"])
    if completed["status"] == "succeeded":
        print(completed["answer"])
finally:
    client.logout()
    client.close()
```

Keep the client open while polling or downloading files. Always call
`logout()` and `close()` in a `finally` block.

## Errors and retries

The client reports safe API errors and exits with a nonzero status when a job
fails.

- Recheck the username and password after an authentication error.
- Respect `Retry-After` when the service asks you to wait.
- A `404` means the requested job or conversation is unavailable to the
  current account.
- A `409` may mean the conversation already has an unfinished turn.
- A local polling timeout does not cancel the submitted job.

Polling requests are safe to repeat. Do not automatically repeat a job
submission when the connection fails before returning a definite result: the
original job may already have been accepted, and another submission could
create a duplicate.
