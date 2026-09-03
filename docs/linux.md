# Linux setup and examples

These commands target Ubuntu and Debian. Run them in a terminal as a normal
user; use `sudo` only for installing system packages.

## 1. Install all dependencies

```bash
sudo apt-get update
sudo apt-get install \
  git python3 python3-pip python3-venv \
  cmake gcc g++ \
  libcurl4-openssl-dev libjson-c-dev nlohmann-json3-dev
```

The Python client uses `requests`. The C++ client uses libcurl and
nlohmann-json. The C client uses libcurl and json-c.

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
python -m pip install -r requirements.txt
```

Activate the environment again with `source .venv/bin/activate` after opening
a new terminal.

## 4. Build C++

```bash
cmake -S cpp -B cpp/build -DCMAKE_BUILD_TYPE=Release
cmake --build cpp/build --parallel
```

The executable is `./cpp/build/jv_api_example`.

## 5. Build C

```bash
cmake -S c -B c/build -DCMAKE_BUILD_TYPE=Release
cmake --build c/build --parallel
```

The executable is `./c/build/jv_api_example`.

## 6. Text-only requests

Do not add `--file` when the request has no attachment.

Python:

```bash
python jv_api_example.py "Explain recursion in simple terms."
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
python jv_api_example.py \
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
python jv_api_example.py \
  "Compare these documents." \
  --file "/home/your-user/Documents/report one.pdf" \
  --file "/home/your-user/Documents/report two.pdf"
```

## Accounts and passwords

The username defaults to `test`. Select another user with `--username`:

```bash
python jv_api_example.py "Return a short status." --username your-username
```

The client asks for the password without displaying it. For approved
automation, let a secret manager populate `JV_API_USERNAME` and
`JV_API_PASSWORD` only for the client process. Do not put credentials in the
repository, a command-line argument, `.env`, logs, or shell history.

## Troubleshooting

- Run commands from the repository root so `./examples/sample-document.txt`
  resolves correctly.
- `No such file or directory` usually means the current directory or file
  path is wrong.
- If CMake cannot find a library, rerun the dependency installation command
  and configure a new build directory.
- Never bypass HTTPS certificate verification. Fix the computer clock, CA
  certificates, proxy, or network instead.
- A polling timeout does not cancel the server-side job. Do not automatically
  submit the same prompt again after an uncertain POST result.

Return to the [main guide](../README.md).
