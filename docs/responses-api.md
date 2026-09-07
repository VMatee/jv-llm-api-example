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
- Input is text-only. Files, generated-file downloads, and legacy conversation
  IDs remain on `/v1/jobs`.
- There is no SSE, synchronous wait, cancellation, usage object, hidden
  reasoning, or full SDK compatibility in this pilot.
- Tool output appears only after terminal provider success and strict server
  validation. A validation failure is terminal and cannot trigger an automatic
  repair or resend.
- A response continuation is owner-scoped and must follow the latest completed
  response in that structured conversation.

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
