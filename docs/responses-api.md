# JV structured Responses API pilot

JV Server exposes an additive, asynchronous compatibility layer at:

```text
POST /v1/responses
GET  /v1/responses/{response_id}
```

It follows the useful parts of the Responses object model while preserving
JV's long-running queue and browser-provider architecture. It is a deliberate
pilot subset, not a drop-in implementation of every OpenAI Responses feature.

## Authentication and submission

First exchange a username and password at `/v1/auth/login`. Use the returned
bearer token for every Responses request. Session cookies alone are not
accepted. Every POST requires these headers:

```http
Authorization: Bearer <token>
Content-Type: application/json
X-JV-CSRF: 1
Idempotency-Key: <unique key for this logical inference round>
```

A basic request is:

```json
{
  "model": "jv-ai",
  "background": true,
  "input": [{"role": "user", "content": "Explain recursion."}],
  "tools": [],
  "tool_choice": "none",
  "parallel_tool_calls": false,
  "store": true,
  "stream": false
}
```

New work returns HTTP `202`; an exact idempotent replay may return `200`.
Save the opaque `id`, then poll its `Location` about every three seconds.
Statuses are `queued`, `in_progress`, `completed`, and `failed`. Nonterminal
and failed responses expose no output.

## Text output

A completed text response contains exactly one assistant message:

```json
{
  "type": "message",
  "id": "msg_opaque",
  "status": "completed",
  "role": "assistant",
  "content": [{
    "type": "output_text",
    "text": "Recursion is...",
    "annotations": []
  }]
}
```

The examples reject unknown statuses, mismatched IDs, output on unfinished
responses, and malformed or multiple completed output items.

## Client-executed tool continuation

Declare tools as strict JSON schemas. A completed response may contain one
validated call:

```json
{
  "type": "function_call",
  "id": "fc_opaque",
  "call_id": "call_opaque",
  "status": "completed",
  "name": "get_client_platform",
  "arguments": "{}"
}
```

Only after terminal completion should the client check its local allowlist,
validate the arguments again, apply its sandbox and approval policy, and run
the tool. The server does not execute local tools. Persist processed `call_id`
values before side effects so repeated polling cannot execute a call twice.

Continue with a new idempotency key and the exact published `call_id`:

```json
{
  "model": "jv-ai",
  "background": true,
  "previous_response_id": "response_opaque",
  "instructions": "Use the trusted tool result and answer the request.",
  "input": [{
    "type": "function_call_output",
    "call_id": "call_opaque",
    "output": "linux"
  }],
  "tools": [],
  "tool_choice": "none",
  "parallel_tool_calls": false,
  "store": true,
  "stream": false
}
```

Instructions and tool definitions are not inherited. Resend them when another
tool round is allowed. Tool results are untrusted input to inference.

## Current boundaries

- `background:true`, `store:true`, `stream:false`, and
  `parallel_tool_calls:false` are required.
- `model` is the generic `jv-ai`; clients cannot override provider, model, or
  effort.
- Input supports text, images and staged files. Generated-file downloads and
  legacy conversation IDs remain on `/v1/jobs`.
- There is no SSE, synchronous wait, cancellation, usage object, hidden
  reasoning, or full SDK compatibility in this pilot.
- Tool output appears only after terminal provider success and strict server
  validation. A validation failure is terminal and cannot trigger an automatic
  repair or resend.
- A response continuation is owner-scoped and must follow the latest completed
  response in that structured conversation.

## Structured attachments

Stage one file with `POST /v1/files`, multipart part `file`, bearer auth,
`X-JV-CSRF: 1` and a separate upload `Idempotency-Key`. Let the HTTP library
generate the multipart boundary. The returned safe object contains `id`,
`object:"file"`, `bytes`, `filename`, `media_type`, `created_at`, `expires_at`.
Its ID is opaque, immutable and owner-scoped; no server path or digest is public.

User content can contain ordered parts:

```json
[
  {"type":"input_text","text":"Compare the attachments."},
  {"type":"input_image","image_url":"data:image/png;base64,<encoded bytes>","detail":"auto"},
  {"type":"input_file","file_id":"file_opaque"}
]
```

Replace placeholders with actual bytes and the returned ID. Attachment-only
user content is valid. References remain associated with their message and
content positions; identical bytes/type may share a provider upload. Ordinary
tool continuation sends only `function_call_output` and `previous_response_id`,
without re-uploading the original files/images.

| Format | MIME |
|---|---|
| PNG/JPEG/WebP | image/png, image/jpeg, image/webp |
| TXT/Markdown | text/plain, text/markdown |
| PDF | application/pdf |
| JSON/CSV/Python `.py` | application/json, text/csv, text/x-python |
| DOCX | application/vnd.openxmlformats-officedocument.wordprocessingml.document |
| XLSX | application/vnd.openxmlformats-officedocument.spreadsheetml.sheet |
| PPTX | application/vnd.openxmlformats-officedocument.presentationml.presentation |

Images: detail `auto` or `high` only; 4 images, 5 MiB each, 12 MiB total decoded;
6,990,508 base64 characters/image; 17 MiB Responses request ceiling with images.
Files: 4 references, 10 MiB each, 20 MiB total. Combined: 6 attachments, 24 MiB.
One staged file per 11 MiB multipart request. Owner staging quota: 10 files /
50 MiB. Staging TTL: 2 hours; canonical attachment context: fixed 24-hour window.
Active work is protected during cleanup; expired context is rejected, not
silently omitted. Structured metadata remains separately bounded.

Central checks MIME/extension/content and fully validates images. Office support
is a conservative ZIP/XML subset: no macros, embedded objects, encryption,
external relationships, unsafe entries or excessive expansion. Client signature
preflight is not full certification. Uploaded source/formulas are never executed
by JV Server.

Structured attachments currently require a ChatGPT-capable account assignment;
Gemini structured attachments fail closed. Clients cannot override routing.
Standalone HTML/XML, JS/TS/Rust/C/C++/Java/Go and shell/config extensions,
arbitrary archives/executables, SVG, audio/video and macro-enabled Office are
unsupported. Do not disguise extensions. Remote URLs, file_url, file_data,
file:// and server paths are unsupported; no remote fetching occurs.

Coding-agent adapters can implement this JV subset; unmodified Codex, streaming,
hosted tools and MCP are not promised.

## Idempotency and failure handling

Use one key of 1–128 ASCII letters, digits, `_`, `.`, `:`, or `-` per logical
round. Retrying byte-for-byte equivalent work with that same key cannot create
another expensive job. A different body with the same key is rejected. A new
continuation round requires a new key.

If a POST times out or disconnects, its outcome is uncertain. Reconcile the
account state and reuse the saved key for the same request. Do not invent a new
key. GET polling can be retried safely and should respect `Retry-After`. A local
poll timeout leaves server work running.

On `failed`, inspect the safe `error.code` and `error.message`; `output` remains
empty. Raw browser-provider protocol text and hidden reasoning are never public.
