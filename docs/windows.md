# Windows setup and examples

Use 64-bit Windows 10 or 11 and PowerShell. Restart PowerShell after installing
tools so updated `PATH` settings are available.

## 1. Install developer tools

Install these components from their official pages:

- [Git for Windows](https://git-scm.com/install/windows)
- [Python for Windows](https://www.python.org/downloads/windows/), version
  3.10 or newer; enable the installer option that adds Python to `PATH`
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
  with the **Desktop development with C++** workload
- [CMake](https://cmake.org/download/); enable its installer option that adds
  CMake to `PATH`

Confirm the tools in a new PowerShell window:

```powershell
git --version
py --version
cmake --version
```

## 2. Install C and C++ libraries with vcpkg

Follow the official [vcpkg setup](https://learn.microsoft.com/vcpkg/get_started/get-started),
or use these PowerShell commands:

```powershell
New-Item -ItemType Directory -Force C:\dev | Out-Null
git clone https://github.com/microsoft/vcpkg.git C:\dev\vcpkg
C:\dev\vcpkg\bootstrap-vcpkg.bat
C:\dev\vcpkg\vcpkg.exe install `
  curl:x64-windows `
  json-c:x64-windows `
  nlohmann-json:x64-windows
```

The examples below assume vcpkg is in `C:\dev\vcpkg`. Change the paths if you
installed it elsewhere.

## 3. Download the examples

```powershell
Set-Location $HOME
git clone https://github.com/VMatee/jv-llm-api-example.git
Set-Location .\jv-llm-api-example
```

Keep PowerShell in this repository directory for the remaining commands.

## 4. Prepare Python

```powershell
py -3 -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install --upgrade pip
python -m pip install -r requirements.txt
```

If PowerShell blocks the activation script, allow scripts for only the current
PowerShell process, then activate again:

```powershell
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
.\.venv\Scripts\Activate.ps1
```

This process-scoped setting disappears when that PowerShell window closes.

## 5. Build C++

```powershell
cmake -S cpp -B cpp/build -A x64 `
  -DCMAKE_TOOLCHAIN_FILE=C:/dev/vcpkg/scripts/buildsystems/vcpkg.cmake
cmake --build cpp/build --config Release
```

The executable is `.\cpp\build\Release\jv_api_example.exe`.

## 6. Build C

```powershell
cmake -S c -B c/build -A x64 `
  -DCMAKE_TOOLCHAIN_FILE=C:/dev/vcpkg/scripts/buildsystems/vcpkg.cmake
cmake --build c/build --config Release
```

The executable is `.\c\build\Release\jv_api_example.exe`.

## 7. Text-only requests

Do not add `--file` when the request has no attachment.

Python:

```powershell
python .\jv_api_example.py "Explain recursion in simple terms."
```

C++:

```powershell
.\cpp\build\Release\jv_api_example.exe "Explain recursion in simple terms."
```

C:

```powershell
.\c\build\Release\jv_api_example.exe "Explain recursion in simple terms."
```

## 8. Requests with an attachment

The repository includes a safe sample document. Add `--file` and its path:

Python:

```powershell
python .\jv_api_example.py `
  "Summarize the attached document." `
  --file .\examples\sample-document.txt
```

C++:

```powershell
.\cpp\build\Release\jv_api_example.exe `
  "Summarize the attached document." `
  --file .\examples\sample-document.txt
```

C:

```powershell
.\c\build\Release\jv_api_example.exe `
  "Summarize the attached document." `
  --file .\examples\sample-document.txt
```

Replace the sample path with your file. Repeat `--file PATH` to attach
multiple files. Put paths containing spaces inside quotes:

```powershell
python .\jv_api_example.py `
  "Compare these documents." `
  --file "$HOME\Documents\report one.pdf" `
  --file "$HOME\Documents\report two.pdf"
```

## Accounts and passwords

The username defaults to `test`. Select another user with `--username`:

```powershell
python .\jv_api_example.py "Return a short status." --username your-username
```

The client asks for the password without displaying it. For approved
automation, let Windows Credential Manager or another secret manager populate
`JV_API_USERNAME` and `JV_API_PASSWORD` only for the client process. Remove a
temporary password variable afterward with:

```powershell
Remove-Item Env:JV_API_PASSWORD -ErrorAction SilentlyContinue
```

Do not put credentials in source code, a command-line argument, `.env`, logs,
PowerShell history, or Git.

## Troubleshooting

- Run commands from the repository root so
  `.\examples\sample-document.txt` resolves correctly.
- If `py`, `git`, or `cmake` is not recognized, reopen PowerShell and verify
  the tool was added to `PATH`.
- If CMake cannot find libcurl or a JSON library, confirm the `x64-windows`
  packages are installed and the vcpkg toolchain path is correct.
- If the executable is not in `Release`, check the selected CMake generator
  and the output printed by `cmake --build`.
- Never bypass HTTPS certificate verification. Fix the computer clock, CA
  certificates, proxy, or network instead.
- A polling timeout does not cancel the server-side job. Do not automatically
  submit the same prompt again after an uncertain POST result.

Return to the [main guide](../README.md).
