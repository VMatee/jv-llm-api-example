# JV LLM API C++ example

This C++17 client demonstrates the safe core JV LLM API workflow:

1. read a password without displaying it;
2. exchange the username/password for a temporary bearer token;
3. submit one prompt and optional files;
4. poll the accepted job until it becomes terminal;
5. print the answer or complete public job JSON;
6. revoke the temporary token.

The client never accepts a password or bearer token as a command-line option,
never prints them, verifies HTTPS certificates, and does not automatically
repeat an ambiguous job-submission POST.

Provider, model, and reasoning mode are deliberately absent. The server uses
the authenticated user's administrator-managed assignment. A normal API user
cannot force ChatGPT, Gemini, a model, or a fallback route.

Complete platform setup: [Linux](../docs/linux.md) ·
[macOS](../docs/macos.md) · [Windows](../docs/windows.md)

## Dependencies

- a C++17 compiler;
- CMake 3.16 or newer;
- libcurl development files;
- nlohmann-json 3.2.0 or newer.

Ubuntu/Debian:

```bash
sudo apt-get update
sudo apt-get install cmake g++ libcurl4-openssl-dev nlohmann-json3-dev
```

macOS with Homebrew:

```bash
brew install cmake curl nlohmann-json
```

Windows with vcpkg:

```powershell
vcpkg install curl nlohmann-json
cmake -S cpp -B cpp/build `
  -DCMAKE_TOOLCHAIN_FILE=C:/path/to/vcpkg/scripts/buildsystems/vcpkg.cmake
cmake --build cpp/build --config Release
```

## Build

From the repository root:

```bash
cmake -S cpp -B cpp/build -DCMAKE_BUILD_TYPE=Release
cmake --build cpp/build --parallel
```

On Windows, the Release executable is normally:

```powershell
.\cpp\build\Release\jv_api_example.exe --help
```

## Example 1: without an attachment

Put the question first and do not add `--file`:

```bash
./cpp/build/jv_api_example "Explain recursion in simple terms."
```

Windows PowerShell:

```powershell
.\cpp\build\Release\jv_api_example.exe "Explain recursion in simple terms."
```

The default username is `test`. Use another account with:

```bash
./cpp/build/jv_api_example \
  "Return a concise project status." \
  --username your-username
```

The program asks for the password without echoing it. The client sends only
the question as the job input.

## Example 2: with an attachment

Add `--file` followed by the file path. This copy-paste example uses the safe
sample document included in the repository:

```bash
./cpp/build/jv_api_example \
  "Summarize the attached document." \
  --file ./examples/sample-document.txt
```

To attach your own files, replace the sample path. Repeat `--file` for more
than one attachment:

```bash
./cpp/build/jv_api_example \
  "Compare these two reports." \
  --file ./report-one.pdf \
  --file ./report-two.pdf
```

## Continue a conversation

The completed result prints its conversation ID. Send only the new prompt and
new files on a follow-up:

```bash
./cpp/build/jv_api_example \
  "Now list the three most important actions." \
  --conversation-id YOUR_CONVERSATION_ID
```

Do not submit another follow-up while the previous turn remains unfinished.

## JSON output

```bash
./cpp/build/jv_api_example "Return a short status." --json
```

## Non-interactive use

For controlled automation, load credentials from an approved secret manager
into temporary environment variables:

```bash
export JV_API_USERNAME="your-username"
read -rsp "JV LLM password: " JV_API_PASSWORD
export JV_API_PASSWORD

./cpp/build/jv_api_example "Return a concise status summary."

unset JV_API_PASSWORD
```

Use `JV_API_BASE_URL` only for an approved deployment. HTTPS is required except
for loopback development addresses:

```bash
export JV_API_BASE_URL="https://your-approved-api.example"
```

Never store credentials in source code, `.env` files, command-line arguments,
URLs, Git history, or application logs.

## Options

```text
jv_api_example QUESTION [options]

--file PATH                 Attach a file; repeat for multiple files
--conversation-id ID        Continue an owned conversation
--base-url URL              Override the API origin
--username USERNAME         Override the default username
--poll-interval SECONDS     Time between status checks; default: 2
--wait-timeout SECONDS      Local polling timeout; default: 3600
--json                      Print the complete safe public terminal-job JSON
```

Polling GET requests are safe to repeat. A polling timeout does not cancel the
server-side job. Never automatically repeat a submission after an uncertain
network result because the first request may already have been accepted.
