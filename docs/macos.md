# macOS setup and examples

These commands work in the default Terminal or another `zsh` terminal on
Apple Silicon and Intel Macs.

## 1. Install developer tools

Install Apple's command-line developer tools:

```bash
xcode-select --install
```

Install [Homebrew](https://brew.sh/) if `brew --version` is unavailable. Then
install the required tools and libraries:

```bash
brew update
brew install git python cmake curl json-c nlohmann-json
```

## 2. Download the examples

```bash
git clone https://github.com/VMatee/jv-llm-api-example.git
cd jv-llm-api-example
```

Keep the terminal in this repository directory for the remaining commands.

## 3. Prepare Python

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install -r python/requirements.txt
```

Activate the environment again with `source .venv/bin/activate` after opening
a new terminal.

## 4. Build C++

Give CMake the Homebrew package locations explicitly so the same command
works with `/opt/homebrew` on Apple Silicon and `/usr/local` on Intel:

```bash
JV_CPP_PREFIX="$(brew --prefix);$(brew --prefix curl);$(brew --prefix nlohmann-json)"
cmake -S cpp -B cpp/build \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_PREFIX_PATH="$JV_CPP_PREFIX"
cmake --build cpp/build --parallel
unset JV_CPP_PREFIX
```

The executable is `./cpp/build/jv_api_example`.

## 5. Build C

```bash
JV_C_PREFIX="$(brew --prefix);$(brew --prefix curl);$(brew --prefix json-c)"
cmake -S c -B c/build \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_PREFIX_PATH="$JV_C_PREFIX"
cmake --build c/build --parallel
unset JV_C_PREFIX
```

The executable is `./c/build/jv_api_example`.

## 6. Text-only requests

Do not add `--file` when the request has no attachment.

Python:

```bash
python ./python/jv_api_example.py "Explain recursion in simple terms."
```

C++:

```bash
./cpp/build/jv_api_example "Explain recursion in simple terms."
```

C:

```bash
./c/build/jv_api_example "Explain recursion in simple terms."
```

## 7. Requests with an attachment

The repository includes a safe sample document. Add `--file` and its path:

Python:

```bash
python ./python/jv_api_example.py \
  "Summarize the attached document." \
  --file ./examples/sample-document.txt
```

C++:

```bash
./cpp/build/jv_api_example \
  "Summarize the attached document." \
  --file ./examples/sample-document.txt
```

C:

```bash
./c/build/jv_api_example \
  "Summarize the attached document." \
  --file ./examples/sample-document.txt
```

Replace the sample path with your file. Repeat `--file PATH` to attach
multiple files. Quote paths containing spaces:

```bash
python ./python/jv_api_example.py \
  "Summarize this document." \
  --file "$HOME/Documents/annual report.pdf"
```

## Accounts and passwords

The username defaults to `test`. Select another user with `--username`:

```bash
python ./python/jv_api_example.py "Return a short status." --username your-username
```

The client asks for the password without displaying it. For approved
automation, let Keychain or another secret manager populate `JV_API_USERNAME`
and `JV_API_PASSWORD` only for the client process. Do not put credentials in
the repository, a command-line argument, `.env`, logs, or shell history.

## Troubleshooting

- Run commands from the repository root so `./examples/sample-document.txt`
  resolves correctly.
- If `brew` is not found after installation, follow Homebrew's displayed
  shell-environment instruction and reopen Terminal.
- If CMake finds an Apple system library instead of Homebrew, delete only the
  affected `c/build` or `cpp/build` directory and rerun the documented CMake
  command with `CMAKE_PREFIX_PATH`.
- Never bypass HTTPS certificate verification. Fix the computer clock, CA
  certificates, proxy, or network instead.
- A polling timeout does not cancel the server-side job. Do not automatically
  submit the same prompt again after an uncertain POST result.

Return to the [main guide](../README.md).
