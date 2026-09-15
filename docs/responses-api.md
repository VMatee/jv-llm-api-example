# JV Responses API reference

Use the asynchronous Responses API to submit text and supported attachments, poll for results, and continue with client-side tool results.

```text
POST /v1/responses
GET  /v1/responses/{response_id}
```

This reference describes the supported JV API subset. Account capabilities and deployment limits apply; unsupported features are rejected rather than silently enabled.

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

### Image-bearing function results (`view_image`)

The string-valued `function_call_output` above remains supported unchanged.
A client-side `view_image` function can return an image using the following
structured result format:

```json
{
  "model": "jv-ai",
  "background": true,
  "previous_response_id": "response_that_published_the_call",
  "input": [{
    "type": "function_call_output",
    "call_id": "call_opaque",
    "output": [{
      "type": "input_image",
      "image_url": "data:image/png;base64,<actual bytes>",
      "detail": "high"
    }]
  }],
  "tools": [],
  "tool_choice": "none",
  "parallel_tool_calls": false,
  "store": true,
  "stream": false
}
```

The result must be the only input item and must use the exact `call_id` from
the latest completed, owned response selected by `previous_response_id`.
Arrays contain 1–4 `input_image` items only. PNG, JPEG, and WebP data URLs are
accepted with `detail:"auto"` or `"high"`; omitted detail becomes `auto`.
Text, files, audio, encrypted content, remote URLs, local/server paths, unknown
items, and mixed unsupported arrays are rejected. This does not certify
arbitrary Responses content arrays.

Images use the same validation and private attachment lifecycle as ordinary
`input_image`: strict canonical base64, MIME/container agreement, complete
single-frame decode, at most 8192 pixels per axis and 16 million pixels, 5 MiB
per decoded image, 12 MiB total, and four images. Confirm cumulative
conversation limits with your service administrator before processing longer histories.
Do not assume that earlier image content stays available indefinitely. If your
application needs to inspect an image again, return it through a new
`view_image` call/result and respect the service limits. The complete image-bearing request is capped at 17 MiB;
normalized structured metadata remains capped at 64 KiB. The general combined
attachment limits below also apply.

`view_image` runs on the client under your application's filesystem permissions.
The service receives only the submitted result, not unrestricted access to the
client filesystem. Keep local image data and saved request state private.

An exact retry uses the same owner-scoped idempotency key, body, call ID, and
image bytes and returns the existing response. Changed bytes or any changed
logical body with the same key returns a conflict. A new continuation requires
a new key.

### Custom/freeform tool (`apply_patch`)

The supported custom tool is `apply_patch`. Use the exact declaration and
grammar below; arbitrary custom grammars are not supported.

```json
{
  "type": "custom",
  "name": "apply_patch",
  "description": "The `apply_patch` tool can be used to edit files. This is a FREEFORM tool, so do not wrap the patch in JSON.",
  "format": {
    "type": "grammar",
    "syntax": "lark",
    "definition": "start: begin_patch hunk+ end_patch\nbegin_patch: \"*** Begin Patch\" LF\nend_patch: \"*** End Patch\" LF?\n\nhunk: add_hunk | delete_hunk | update_hunk\nadd_hunk: \"*** Add File: \" filename LF add_line+\ndelete_hunk: \"*** Delete File: \" filename LF\nupdate_hunk: \"*** Update File: \" filename LF change_move? change?\n\nfilename: /(.+)/\nadd_line: \"+\" /(.*)/ LF -> line\n\nchange_move: \"*** Move to: \" filename LF\nchange: (change_context | change_line)+ eof_line?\nchange_context: (\"@@\" | \"@@ \" /(.+)/) LF\nchange_line: (\"+\" | \"-\" | \" \") /(.*)/ LF\neof_line: \"*** End of File\" LF\n\n%import common.LF\n"
  }
}
```

Use the exact `definition` string; line endings and final
newline are significant. JV rejects another grammar digest, syntax, format,
unknown custom-tool fields or undeclared name. JSON function tools and this
custom tool share the existing limit of 16 unique declarations. Only this `apply_patch` form is supported; this does not imply general custom
tool compatibility.

A completed response can return a validated call in this format:

```json
{
  "type": "custom_tool_call",
  "id": "ctc_opaque",
  "call_id": "call_opaque",
  "name": "apply_patch",
  "input": "*** Begin Patch\n*** Add File: example.txt\n+example\n*** End Patch"
}
```

The client must preserve the opaque `call_id` and freeform `input` exactly,
then validate the patch and apply its own sandbox and approval policy. The
freeform input is data, not JSON arguments. It is nonempty UTF-8 and capped at
32 KiB. **JV Server never executes `apply_patch`, parses source files, or
applies a patch.**

After client execution, submit the corresponding result as the sole input with
a new key and the exact latest predecessor:

```json
{
  "model": "jv-ai",
  "background": true,
  "previous_response_id": "response_that_published_the_custom_call",
  "input": [{
    "type": "custom_tool_call_output",
    "call_id": "call_opaque",
    "output": "Success. Updated the following files:\nA example.txt"
  }],
  "tools": [],
  "tool_choice": "none",
  "parallel_tool_calls": false,
  "store": true,
  "stream": false
}
```

The custom result is a UTF-8 string capped at 32 KiB; an array is unsupported.
Its result type must match the pending custom call class and its exact call ID.
Owner scope, latest-completed ordering, idempotency, and one nonterminal turn
remain enforced. Exact same-key replay returns the existing response; changed
result text, including whitespace, conflicts. Resend declarations and choose
`auto` or `required` only when another tool call should be permitted.

## Supported features and limits

- `background:true`, `store:true`, `stream:false`, and
  `parallel_tool_calls:false` are required.
- `model` is the generic `jv-ai`; service options are controlled by the account
  assignment and cannot be overridden in the request.
- Input supports text, images, staged files, JSON function tools, the certified
  image-bearing function result, and the certified custom `apply_patch` flow.
  Generated-file downloads and legacy conversation IDs remain on `/v1/jobs`.
- There is no SSE, synchronous wait, cancellation, usage object, hidden
  reasoning, or full SDK compatibility in this pilot.
- Tool output appears only after successful completion and response
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
user content is valid. Attachment order and message association are preserved. Ordinary
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

Images: detail `auto` or `high` only; 4 images,
5 MiB each, 12 MiB total decoded;
6,990,508 base64 characters/image; 17 MiB Responses request ceiling with images.
Files: 4 references, 10 MiB each, 20 MiB total. Combined: 6 attachments, 24 MiB.
One staged file per 11 MiB multipart request. Owner staging quota: 10 files /
50 MiB. Staging TTL: 2 hours; canonical attachment context: fixed 24-hour window.
Active work is protected during cleanup; expired context is rejected, not
silently omitted. Structured metadata remains separately bounded.

The service checks MIME/extension/content and validates images. Office support
is a conservative ZIP/XML subset: no macros, embedded objects, encryption,
external relationships, unsafe entries or excessive expansion. Client signature
preflight is not full certification. Uploaded source/formulas are never executed
by JV Server.

Structured attachments require a compatible account capability. If the service
rejects an attachment capability, ask your administrator; clients cannot override
account settings.
Standalone HTML/XML, JS/TS/Rust/C/C++/Java/Go and shell/config extensions,
arbitrary archives/executables, SVG, audio/video and macro-enabled Office are
unsupported. Do not disguise extensions. Remote URLs, file_url, file_data,
file:// and server paths are unsupported; no remote fetching occurs.

Use only the documented request and response shapes. Unsupported content,
streaming, parallel calls, and unlisted tool formats are not supported by this API.

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
empty. Use the returned error code and message for diagnostics; do not depend on
undocumented response fields.
