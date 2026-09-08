# Verification scope — 2026-09-08

This records bounded checks, not a claim that software is perfect or that all
Codex/OpenAI protocol features are supported.

- Server's full preceding regression: 474 central tests passed, 1 skipped;
  258 ChatGPT and 271 Gemini provider tests passed.
- Server production acceptance: source/data (Python + JSON), Office
  (DOCX + XLSX + PPTX), and mixed image + CSV + DOCX through a client tool
  continuation. Three flows, four inference rounds; original attachment-only
  facts were recovered and actual image relationships were interpreted.
- Private canonical audit: all 24 existing structured request records verified
  against their stored request/context and successful answer content; no
  integrity issues. Retention/expiry rules still apply. Rejected HTTP requests
  are not necessarily inference/history records.
- Public Python client: 6 offline tests passed, including real loopback HTTP
  staging, mixed content, polling and continuation.
- Public Rust client: 28 offline tests passed; 1 live integration test intentionally
  ignored; Clippy with warnings denied passed.
- Provider/browser implementations and JV CLI were not changed. No additional
  provider inference was spent merely to publish the client examples.

The public clients have offline HTTP proof; the live server attachment proof
used the internal Rust acceptance client, not these public binaries. Native
Codex integration and unsupported attachment families remain separate work.
Production credentials, request bytes and private logs are excluded from Git.
