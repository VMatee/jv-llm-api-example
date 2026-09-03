# JV LLM API Python guide

The Python 3.10 client is the fullest reference implementation. It supports
prompts, multiple attachments, conversation follow-ups, polling, complete JSON
output, and verified response-file downloads.

Provider, model, and reasoning controls are intentionally absent. The server
uses the authenticated user's administrator-managed assignment.

## Install

Clone the repository and create an isolated environment:

```bash
git clone https://github.com/VMatee/jv-llm-api-example.git
cd jv-llm-api-example
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r requirements.txt
```

Windows PowerShell:

```powershell
git clone https://github.com/VMatee/jv-llm-api-example.git
cd jv-llm-api-example
python -m venv .venv
.venv\Scripts\Activate.ps1
python -m pip install -r requirements.txt
```

## Ask a question

```bash
python jv_api_example.py "Explain recursion in simple terms."
```

The default username is `test`. To use another account:

```bash
python jv_api_example.py \
  "Return a concise project status." \
  --username your-username
```

The password prompt does not display the password.

## Attach files

```bash
python jv_api_example.py \
  "Compare these reports." \
  --file ./report-one.pdf \
  --file ./report-two.pdf
```

## Continue a conversation

Use the conversation ID printed by a completed request:

```bash
python jv_api_example.py \
  "Now list the three most important actions." \
  --conversation-id YOUR_CONVERSATION_ID
```

Send only the new question and new files. The service supplies successful
earlier context. Do not submit a follow-up while the previous turn remains
unfinished.

## Download generated files

```bash
python jv_api_example.py \
  "Create an explanatory image." \
  --download-dir ./results
```

The client validates the authenticated response-file manifest, byte count, and
safe local filename before completing a download.

## Complete JSON

```bash
python jv_api_example.py "Return a short status." --json
```

## Use from Python

```python
import getpass

from jv_api_example import JVAIClient

client = JVAIClient("https://ai.openjvspace.com")
try:
    client.login("test", getpass.getpass("Password: "))
    created = client.submit_job("Explain recursion in simple terms.")
    completed = client.wait_for_job(created["id"])
    if completed["status"] == "succeeded":
        print(completed["answer"])
finally:
    client.logout()
    client.close()
```

## Non-interactive use

Load credentials from an approved secret manager into temporary environment
variables:

```bash
export JV_API_USERNAME="your-username"
read -rsp "JV LLM password: " JV_API_PASSWORD
export JV_API_PASSWORD

python jv_api_example.py "Return a concise status summary."

unset JV_API_PASSWORD
```

Use `JV_API_BASE_URL` only for an approved deployment. HTTPS is mandatory
except for loopback development addresses.

Never place passwords or bearer tokens in source code, `.env` files,
command-line arguments, URLs, Git history, or application logs.

## Options

```text
python jv_api_example.py QUESTION [options]

--file PATH                 Attach a file; repeat for multiple files
--conversation-id ID        Continue an owned conversation
--base-url URL              Override the API origin
--username USERNAME         Override the default username
--poll-interval SECONDS     Time between status checks; default: 2
--wait-timeout SECONDS      Local polling timeout; default: 3600
--json                      Print complete public job JSON
--download-dir DIRECTORY    Download generated response files
```

Polling is safe to repeat. A local timeout does not cancel the server job.
Never automatically repeat an uncertain submission because the first POST may
already have been accepted.
