# JV LLM API C guide

This C11 client demonstrates the safe core JV LLM API workflow using libcurl
and json-c:

1. read a password without displaying it;
2. exchange the username/password for a temporary bearer token;
3. submit a prompt and optional files;
4. poll until the accepted job becomes terminal;
5. print the answer or complete public job JSON;
6. revoke the temporary token.

The client verifies HTTPS certificates, never accepts credentials on the
command line, and never automatically repeats an ambiguous submission.
Provider/model overrides are intentionally unavailable to normal users.

Complete platform setup: [Linux](../docs/linux.md) ·
[macOS](../docs/macos.md) · [Windows](../docs/windows.md)

## Dependencies

- a C11 compiler;
- CMake 3.16 or newer;
- libcurl development files;
- json-c development files.

Ubuntu/Debian:

```bash
sudo apt-get update
sudo apt-get install cmake gcc libcurl4-openssl-dev libjson-c-dev
```

macOS with Homebrew:

```bash
brew install cmake curl json-c
```

Windows with vcpkg:

```powershell
vcpkg install curl json-c
cmake -S c -B c/build `
  -DCMAKE_TOOLCHAIN_FILE=C:/path/to/vcpkg/scripts/buildsystems/vcpkg.cmake
cmake --build c/build --config Release
```

## Build

From the repository root:

```bash
cmake -S c -B c/build -DCMAKE_BUILD_TYPE=Release
cmake --build c/build --parallel
```

On Windows, the Release executable is normally:

```powershell
.\c\build\Release\jv_api_example.exe --help
```

## Example 1: without an attachment

Put the question first and do not add `--file`:

```bash
./c/build/jv_api_example "Explain recursion in simple terms."
```

Windows PowerShell:

```powershell
.\c\build\Release\jv_api_example.exe "Explain recursion in simple terms."
```

The username defaults to `test`. Use another account with:

```bash
./c/build/jv_api_example \
  "Return a concise project status." \
  --username your-username
```

The client sends only the question as the job input.

## Example 2: with an attachment

Add `--file` followed by the file path. This copy-paste example uses the safe
sample document included in the repository:

```bash
./c/build/jv_api_example \
  "Summarize the attached document." \
  --file ./examples/sample-document.txt
```

To attach your own files, replace the sample path. Repeat `--file` for more
than one attachment:

```bash
./c/build/jv_api_example \
  "Compare these reports." \
  --file ./report-one.pdf \
  --file ./report-two.pdf
```

## Continue a conversation

```bash
./c/build/jv_api_example \
  "Now list the three most important actions." \
  --conversation-id YOUR_CONVERSATION_ID
```

Send only the new prompt and new files. Do not submit another follow-up while
the previous turn remains unfinished.

## Complete JSON

```bash
./c/build/jv_api_example "Return a short status." --json
```

## Non-interactive use

Use an approved secret manager and temporary environment variables:

```bash
export JV_API_USERNAME="your-username"
read -rsp "JV LLM password: " JV_API_PASSWORD
export JV_API_PASSWORD

./c/build/jv_api_example "Return a concise status summary."

unset JV_API_PASSWORD
```

`JV_API_BASE_URL` may select another approved deployment. HTTPS is mandatory
except for loopback development addresses.

Never place credentials in source code, `.env` files, command-line arguments,
URLs, Git history, or logs.

## Options

```text
jv_api_example QUESTION [options]

--file PATH                 Attach a file; repeat for multiple files
--conversation-id ID        Continue an owned conversation
--base-url URL              Override the API origin
--username USERNAME         Override the default username
--poll-interval SECONDS     Time between status checks; default: 2
--wait-timeout SECONDS      Local polling timeout; default: 3600
--json                      Print complete public job JSON
```

Polling is safe to repeat. A timeout does not cancel the server-side job.
Never automatically repeat an uncertain submission because the first POST may
already have been accepted.
