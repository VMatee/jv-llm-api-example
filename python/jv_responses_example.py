#!/usr/bin/env python3
"""Use JV's asynchronous structured Responses API pilot safely."""

from __future__ import annotations

import argparse
import getpass
import json
import os
import platform
import secrets
import sys
import time
from typing import Any, Callable

import requests

try:
    from .attachments import image_part, local_file
    from .jv_api_example import (
        CSRF_HEADER,
        CSRF_VALUE,
        DEFAULT_BASE_URL,
        DEFAULT_USERNAME,
        JVAPIError,
        _require_status,
        _validated_base_url,
    )
except ImportError:  # Direct script execution adds python/ instead of its parent.
    from attachments import image_part, local_file
    from jv_api_example import (
        CSRF_HEADER,
        CSRF_VALUE,
        DEFAULT_BASE_URL,
        DEFAULT_USERNAME,
        JVAPIError,
        _require_status,
        _validated_base_url,
    )

TERMINAL_RESPONSE_STATUSES = frozenset({"completed", "failed"})
CLIENT_TOOL_NAME = "get_client_platform"


def _idempotency_key() -> str:
    return f"jv-example-{secrets.token_hex(16)}"


def _require_id(value: Any, field: str) -> str:
    if (
        not isinstance(value, str)
        or not value
        or len(value) > 200
        or not value.isascii()
        or not all(char.isalnum() or char in "-_" for char in value)
    ):
        raise JVAPIError(f"The response has an invalid {field}.")
    return value


def _validate_response(payload: dict[str, Any], expected_id: str | None = None) -> None:
    response_id = _require_id(payload.get("id"), "response ID")
    if expected_id is not None and response_id != expected_id:
        raise JVAPIError("The polling response ID changed unexpectedly.")
    if payload.get("object") != "response":
        raise JVAPIError("The API returned an unexpected object type.")
    status = payload.get("status")
    if status not in {"queued", "in_progress", "completed", "failed"}:
        raise JVAPIError("The API returned an unknown response status.")
    output = payload.get("output")
    if not isinstance(output, list):
        raise JVAPIError("The API returned an invalid output list.")
    if status in {"queued", "in_progress", "failed"} and output:
        raise JVAPIError("An unfinished or failed response exposed output.")
    if status == "completed" and len(output) != 1:
        raise JVAPIError("A completed response must contain exactly one output item.")


def _text_output(payload: dict[str, Any]) -> str:
    _validate_response(payload)
    if payload.get("status") != "completed":
        error = payload.get("error")
        if isinstance(error, dict):
            code = error.get("code", "JV-AGENT")
            message = error.get("message", "The structured response failed.")
            raise JVAPIError(f"{code}: {message}")
        raise JVAPIError("The structured response did not complete.")
    item = payload["output"][0]
    if (
        not isinstance(item, dict)
        or item.get("type") != "message"
        or item.get("role") != "assistant"
        or item.get("status") != "completed"
    ):
        raise JVAPIError("Expected one completed assistant message.")
    content = item.get("content")
    if not isinstance(content, list) or len(content) != 1:
        raise JVAPIError("The assistant message has invalid content.")
    part = content[0]
    if not isinstance(part, dict) or part.get("type") != "output_text":
        raise JVAPIError("The assistant message is not text output.")
    text = part.get("text")
    if not isinstance(text, str):
        raise JVAPIError("The assistant text is invalid.")
    return text


def _tool_call(payload: dict[str, Any]) -> tuple[str, str]:
    _validate_response(payload)
    if payload.get("status") != "completed":
        raise JVAPIError("The tool-request response did not complete.")
    item = payload["output"][0]
    if not isinstance(item, dict) or item.get("type") != "function_call":
        raise JVAPIError("Expected one validated function call.")
    if item.get("status") != "completed" or item.get("name") != CLIENT_TOOL_NAME:
        raise JVAPIError("The server returned an unexpected tool call.")
    call_id = _require_id(item.get("call_id"), "tool call ID")
    arguments_text = item.get("arguments")
    if not isinstance(arguments_text, str):
        raise JVAPIError("Tool arguments must be a JSON string.")
    try:
        arguments = json.loads(arguments_text)
    except ValueError as exc:
        raise JVAPIError("Tool arguments are not valid JSON.") from exc
    if arguments != {}:
        raise JVAPIError("The platform tool accepts no arguments.")
    return call_id, platform.system() or "Unknown"


class JVResponsesClient:
    """Small client for JV's documented Responses-compatible pilot subset."""

    def __init__(self, base_url: str = DEFAULT_BASE_URL, timeout: float = 120.0):
        self.base_url = _validated_base_url(base_url)
        self.timeout = timeout
        self.session = requests.Session()
        self.session.headers.update(
            {
                "Accept": "application/json",
                "User-Agent": "JV-AI-Python-Responses-Example/1.0",
                CSRF_HEADER: CSRF_VALUE,
            }
        )
        self._authenticated = False

    def stage_file(self, path, idempotency_key: str) -> dict[str, Any]:
        if not self._authenticated:
            raise JVAPIError("Call login() before staging a file.")
        name, mime, data = local_file(path)
        try:
            response = self.session.post(
                f"{self.base_url}/v1/files",
                files={"file": (name, data, mime)},
                headers={"Idempotency-Key": idempotency_key},
                timeout=self.timeout,
                allow_redirects=False,
            )
            value = _require_status(response, {200, 201})
            file_id = value.get("id") if isinstance(value, dict) else None
            if (
                not isinstance(file_id, str)
                or len(file_id) != 48
                or not file_id.startswith("file_")
            ):
                raise JVAPIError("Invalid staged file object")
            _require_id(file_id, "file ID")
            if (
                value.get("object") != "file"
                or value.get("bytes") != len(data)
                or value.get("filename") != name
                or value.get("media_type") != mime
            ):
                raise JVAPIError("Invalid staged file metadata")
            return value
        except (requests.RequestException, JVAPIError) as exc:
            raise JVAPIError(
                f"Staging not confirmed; retain the same upload key {idempotency_key} and exact bytes for reconciliation."
            ) from exc

    def login(self, username: str, password: str) -> None:
        response = self.session.post(
            f"{self.base_url}/v1/auth/login",
            json={"username": username, "password": password, "remember_me": False},
            timeout=self.timeout,
            allow_redirects=False,
        )
        payload = _require_status(response, {200})
        token = payload.get("access_token") if payload else None
        if not isinstance(token, str) or not token:
            raise JVAPIError("The login response did not include a bearer token.")
        self.session.headers["Authorization"] = f"Bearer {token}"
        self._authenticated = True

    def create(
        self, body: dict[str, Any], idempotency_key: str | None = None
    ) -> dict[str, Any]:
        if not self._authenticated:
            raise JVAPIError("Call login() before creating a response.")
        key = idempotency_key or _idempotency_key()
        try:
            response = self.session.post(
                f"{self.base_url}/v1/responses",
                json=body,
                headers={"Idempotency-Key": key},
                timeout=self.timeout,
                allow_redirects=False,
            )
        except requests.RequestException as exc:
            raise JVAPIError(
                f"Response submission is uncertain. Poll account state before retrying; "
                f"reuse idempotency key {key}."
            ) from exc
        try:
            payload = _require_status(response, {200, 202})
            if payload is None:
                raise JVAPIError("The create response endpoint returned no JSON.")
            _validate_response(payload)
        except (JVAPIError, ValueError) as exc:
            raise JVAPIError(
                f"Submission not confirmed; retain the exact request and idempotency key {key} for reconciliation."
            ) from exc
        return payload

    def get(self, response_id: str) -> dict[str, Any]:
        _require_id(response_id, "response ID")
        response = self.session.get(
            f"{self.base_url}/v1/responses/{response_id}",
            timeout=self.timeout,
            allow_redirects=False,
        )
        payload = _require_status(response, {200})
        if payload is None:
            raise JVAPIError("The response endpoint returned no JSON.")
        _validate_response(payload, response_id)
        return payload

    def wait(
        self,
        response_id: str,
        poll_interval: float = 3.0,
        wait_timeout: float = 3600.0,
        progress: Callable[[dict[str, Any]], None] | None = None,
    ) -> dict[str, Any]:
        deadline = time.monotonic() + wait_timeout
        last_status: Any = None
        while True:
            payload = self.get(response_id)
            if progress and payload.get("status") != last_status:
                progress(payload)
            last_status = payload.get("status")
            if last_status in TERMINAL_RESPONSE_STATUSES:
                return payload
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise JVAPIError(
                    "Local polling timed out. The server response continues and may be "
                    f"polled with ID {response_id}."
                )
            time.sleep(min(poll_interval, remaining))

    def logout(self) -> None:
        if not self._authenticated:
            return
        try:
            response = self.session.post(
                f"{self.base_url}/v1/auth/logout",
                timeout=self.timeout,
                allow_redirects=False,
            )
            _require_status(response, {204})
        finally:
            self._authenticated = False
            self.session.headers.pop("Authorization", None)

    def close(self) -> None:
        self.session.close()


def _base_request(question: str) -> dict[str, Any]:
    return {
        "model": "jv-ai",
        "background": True,
        "input": [{"role": "user", "content": question}],
        "tools": [],
        "tool_choice": "none",
        "parallel_tool_calls": False,
        "store": True,
        "stream": False,
    }


def _tool_request(question: str) -> dict[str, Any]:
    body = _base_request(question)
    body["instructions"] = (
        "Call get_client_platform once. After its result arrives, answer the user "
        "briefly without requesting another tool."
    )
    body["tools"] = [
        {
            "type": "function",
            "name": CLIENT_TOOL_NAME,
            "description": "Return the operating-system family of this client.",
            "strict": True,
            "parameters": {
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": False,
            },
        }
    ]
    body["tool_choice"] = "required"
    return body


def _continuation(previous_id: str, call_id: str, result: str) -> dict[str, Any]:
    return {
        "model": "jv-ai",
        "background": True,
        "previous_response_id": previous_id,
        "instructions": "Use the trusted tool result and answer the original request.",
        "input": [
            {"type": "function_call_output", "call_id": call_id, "output": result}
        ],
        "tools": [],
        "tool_choice": "none",
        "parallel_tool_calls": False,
        "store": True,
        "stream": False,
    }


class AttachmentAction(argparse.Action):
    def __call__(self, parser, namespace, values, option_string=None):
        items = list(getattr(namespace, self.dest, None) or [])
        items.append((option_string, values))
        setattr(namespace, self.dest, items)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("question")
    parser.add_argument("--tool-demo", action="store_true")
    parser.add_argument(
        "--file", "--image", dest="attachments", action=AttachmentAction
    )
    parser.add_argument("--image-detail", choices=("auto", "high"), default="auto")
    parser.add_argument(
        "--idempotency-key",
        default=None,
        help="Retain this key and exact input for ambiguous submission recovery",
    )
    parser.add_argument("--json", action="store_true")
    parser.add_argument(
        "--base-url", default=os.getenv("JV_API_BASE_URL", DEFAULT_BASE_URL)
    )
    parser.add_argument(
        "--username", default=os.getenv("JV_API_USERNAME", DEFAULT_USERNAME)
    )
    parser.add_argument("--poll-interval", type=float, default=3.0)
    parser.add_argument("--wait-timeout", type=float, default=3600.0)
    args = parser.parse_args()
    if args.poll_interval <= 0 or args.wait_timeout <= 0:
        parser.error("polling values must be positive")

    client: JVResponsesClient | None = None
    try:
        client = JVResponsesClient(args.base_url)
        password = os.getenv("JV_API_PASSWORD")
        if password is None:
            password = getpass.getpass(f"Password for {args.username}: ")
        client.login(args.username, password)
        print(f"Authenticated as {args.username}.", file=sys.stderr)

        body = (
            _tool_request(args.question)
            if args.tool_demo
            else _base_request(args.question)
        )
        key = args.idempotency_key or _idempotency_key()
        if (
            not 1 <= len(key) <= 100
            or not key.isascii()
            or not all(c.isalnum() or c in "_.:-" for c in key)
        ):
            raise JVAPIError("CLI idempotency key must be 1–100 safe ASCII characters")
        print(f"Logical request key: {key}", file=sys.stderr)
        attachments = args.attachments or []
        if (
            len(attachments) > 6
            or sum(k == "--file" for k, _ in attachments) > 4
            or sum(k == "--image" for k, _ in attachments) > 4
        ):
            raise JVAPIError("At most 4 files, 4 images and 6 mixed attachments")
        if attachments:
            parts = (
                [{"type": "input_text", "text": args.question}] if args.question else []
            )
            for index, (kind, path) in enumerate(attachments):
                if kind == "--image":
                    parts.append(image_part(path, args.image_detail))
                else:
                    staged = client.stage_file(path, f"{key}-upload-{index}")
                    parts.append({"type": "input_file", "file_id": staged["id"]})
            body["input"][0]["content"] = parts
        created = client.create(body, key)
        response_id = created["id"]
        print(f"Created structured response {response_id}.", file=sys.stderr)
        terminal = client.wait(
            response_id,
            args.poll_interval,
            args.wait_timeout,
            lambda value: print(f"Status: {value['status']}", file=sys.stderr),
        )
        if args.tool_demo:
            call_id, tool_result = _tool_call(terminal)
            print(
                f"Executing allowlisted local tool {CLIENT_TOOL_NAME}; JV Server does not execute it.",
                file=sys.stderr,
            )
            created = client.create(
                _continuation(response_id, call_id, tool_result), f"{key}-continuation"
            )
            response_id = created["id"]
            print(f"Created continuation response {response_id}.", file=sys.stderr)
            terminal = client.wait(
                response_id,
                args.poll_interval,
                args.wait_timeout,
                lambda value: print(f"Status: {value['status']}", file=sys.stderr),
            )
        answer = _text_output(terminal)
        print(
            json.dumps(terminal, ensure_ascii=False, indent=2) if args.json else answer
        )
        return 0
    except (JVAPIError, requests.RequestException, OSError, ValueError) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("Interrupted. Any submitted server response continues.", file=sys.stderr)
        return 130
    finally:
        if client is not None:
            try:
                client.logout()
            except (JVAPIError, requests.RequestException) as exc:
                print(f"Warning: logout failed: {exc}", file=sys.stderr)
            client.close()


if __name__ == "__main__":
    raise SystemExit(main())
