#!/usr/bin/env python3
"""Call the public JV LLM API with a user account."""

from __future__ import annotations

import argparse
import getpass
import json
import mimetypes
import os
import re
import sys
import time
from contextlib import ExitStack
from pathlib import Path
from typing import Any, Callable, Iterable
from urllib.parse import urlsplit

import requests


DEFAULT_BASE_URL = "https://ai.openjvspace.com"
DEFAULT_USERNAME = "test"
CSRF_HEADER = "X-JV-CSRF"
CSRF_VALUE = "1"
TERMINAL_JOB_STATUSES = frozenset({"succeeded", "failed"})
MAX_RESPONSE_FILE_BYTES = 25 * 1024 * 1024
MAX_RESPONSE_TOTAL_BYTES = 100 * 1024 * 1024
MAX_RESPONSE_FILES = 10


class JVAPIError(RuntimeError):
    """A safe error returned by the JV AI API or its client."""


def _validated_base_url(value: str) -> str:
    candidate = value.strip().rstrip("/")
    parsed = urlsplit(candidate)
    is_loopback_http = parsed.scheme == "http" and parsed.hostname in {
        "127.0.0.1",
        "localhost",
    }
    if (
        parsed.scheme != "https"
        and not is_loopback_http
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
        or parsed.path not in {"", "/"}
    ):
        raise JVAPIError(
            "The API base URL must be an HTTPS origin, or loopback HTTP for "
            "local development."
        )
    return candidate


def _safe_response_error(response: requests.Response) -> JVAPIError:
    code = "JV-HTTP"
    message = f"The JV AI API returned HTTP {response.status_code}."
    try:
        payload = response.json()
    except (requests.RequestException, ValueError):
        payload = None
    if isinstance(payload, dict):
        error = payload.get("error")
        if isinstance(error, dict):
            if isinstance(error.get("code"), str):
                code = error["code"]
            if isinstance(error.get("message"), str):
                message = error["message"]
    retry_after = response.headers.get("Retry-After")
    retry_note = f" Retry after {retry_after} seconds." if retry_after else ""
    return JVAPIError(f"{code}: {message}{retry_note}")


def _require_status(
    response: requests.Response,
    expected_statuses: Iterable[int],
) -> dict[str, Any] | None:
    if response.status_code not in set(expected_statuses):
        raise _safe_response_error(response)
    if response.status_code == 204:
        return None
    try:
        payload = response.json()
    except (requests.RequestException, ValueError) as exc:
        raise JVAPIError("The JV AI API returned invalid JSON.") from exc
    if not isinstance(payload, dict):
        raise JVAPIError("The JV AI API returned an unexpected response.")
    return payload


class JVAIClient:
    """Small reference client for the public JV LLM API."""

    def __init__(
        self,
        base_url: str = DEFAULT_BASE_URL,
        *,
        request_timeout: float = 120.0,
        session: requests.Session | None = None,
    ) -> None:
        if request_timeout <= 0:
            raise ValueError("request_timeout must be positive")
        self.base_url = _validated_base_url(base_url)
        self.request_timeout = request_timeout
        self.session = session or requests.Session()
        self.session.headers.update(
            {
                "Accept": "application/json",
                "User-Agent": "JV-AI-Python-Example/1.0",
                CSRF_HEADER: CSRF_VALUE,
            }
        )
        self._access_token: str | None = None

    def login(self, username: str, password: str) -> dict[str, Any]:
        """Exchange one username/password pair for a temporary bearer token."""
        if not username or not password:
            raise JVAPIError("Username and password are required.")
        try:
            response = self.session.post(
                f"{self.base_url}/v1/auth/login",
                json={
                    "username": username,
                    "password": password,
                    "remember_me": False,
                },
                timeout=self.request_timeout,
            )
        except requests.RequestException as exc:
            raise JVAPIError("Could not reach the JV AI login endpoint.") from exc
        payload = _require_status(response, {200})
        token = payload.get("access_token") if payload is not None else None
        if not isinstance(token, str) or not token:
            raise JVAPIError("The login response did not include a bearer token.")
        self._access_token = token
        self.session.headers["Authorization"] = f"Bearer {token}"
        return payload

    def submit_job(
        self,
        text: str,
        *,
        file_paths: Iterable[Path] = (),
        conversation_id: str | None = None,
    ) -> dict[str, Any]:
        """Submit one new turn without automatically retrying an ambiguous POST."""
        self._require_login()
        if not isinstance(text, str) or not text.strip():
            raise JVAPIError("Question text must not be empty.")

        paths = [Path(path) for path in file_paths]
        for path in paths:
            if not path.is_file() or path.is_symlink():
                raise JVAPIError(f"Attachment is not a regular file: {path}")

        # Supplying text as a multipart part forces requests to use
        # multipart/form-data even when the job has no attachments.
        parts: list[tuple[str, tuple[Any, ...]]] = [("text", (None, text))]
        if conversation_id is not None:
            if not conversation_id.strip():
                raise JVAPIError("conversation_id must not be empty.")
            parts.append(("conversation_id", (None, conversation_id)))

        try:
            with ExitStack() as stack:
                for path in paths:
                    content_type = (
                        mimetypes.guess_type(path.name)[0] or "application/octet-stream"
                    )
                    handle = stack.enter_context(path.open("rb"))
                    parts.append(("files", (path.name, handle, content_type)))
                response = self.session.post(
                    f"{self.base_url}/v1/jobs",
                    files=parts,
                    timeout=self.request_timeout,
                )
        except requests.RequestException as exc:
            raise JVAPIError(
                "Job submission did not return a definite result. Do not "
                "automatically repeat this POST because the first job may "
                "already exist."
            ) from exc

        payload = _require_status(response, {202})
        job_id = payload.get("id") if payload is not None else None
        if not isinstance(job_id, str) or not job_id:
            raise JVAPIError("The job response did not include a job ID.")
        return payload

    def get_job(self, job_id: str) -> dict[str, Any]:
        """Read one job owned by the authenticated account."""
        self._require_login()
        if not job_id:
            raise JVAPIError("job_id is required.")
        try:
            response = self.session.get(
                f"{self.base_url}/v1/jobs/{job_id}",
                timeout=self.request_timeout,
            )
        except requests.RequestException as exc:
            raise JVAPIError("Could not poll the JV AI job.") from exc
        payload = _require_status(response, {200})
        if payload is None:
            raise JVAPIError("The job endpoint returned an empty response.")
        return payload

    def wait_for_job(
        self,
        job_id: str,
        *,
        poll_interval: float = 2.0,
        wait_timeout: float = 3600.0,
        progress: Callable[[dict[str, Any]], None] | None = None,
    ) -> dict[str, Any]:
        """Poll until the job succeeds, fails, or the local wait times out."""
        if poll_interval <= 0 or wait_timeout <= 0:
            raise ValueError("Polling intervals and timeouts must be positive")
        deadline = time.monotonic() + wait_timeout
        last_state: tuple[Any, Any] | None = None
        while True:
            job = self.get_job(job_id)
            state = (job.get("status"), job.get("phase"))
            if progress is not None and state != last_state:
                progress(job)
            last_state = state
            if job.get("status") in TERMINAL_JOB_STATUSES:
                return job
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise JVAPIError(
                    "Local polling timed out. The server-side job was not "
                    "cancelled; it can be polled again using the same job ID."
                )
            time.sleep(min(poll_interval, remaining))

    def download_response_files(
        self,
        job: dict[str, Any],
        destination: Path,
    ) -> list[Path]:
        """Download this job's authenticated generated images/files safely."""
        self._require_login()
        job_id = job.get("id")
        response_value = job.get("response")
        files = (
            response_value.get("files") if isinstance(response_value, dict) else None
        )
        if not isinstance(job_id, str) or not job_id:
            raise JVAPIError("The terminal job has no valid ID.")
        if files is None:
            return []
        if not isinstance(files, list) or len(files) > MAX_RESPONSE_FILES:
            raise JVAPIError("The response file manifest is invalid.")

        destination = Path(destination).expanduser().absolute()
        destination.mkdir(parents=True, exist_ok=True, mode=0o700)
        if not destination.is_dir() or destination.is_symlink():
            raise JVAPIError("The response download destination is not safe.")
        destination = destination.resolve(strict=True)
        downloaded: list[Path] = []
        total_bytes = 0
        expected_prefix = f"/v1/jobs/{job_id}/response-files/"
        for index, item in enumerate(files, start=1):
            if not isinstance(item, dict):
                raise JVAPIError("The response file manifest is invalid.")
            relative_url = item.get("url")
            if (
                not isinstance(relative_url, str)
                or not relative_url.startswith(expected_prefix)
                or "\\" in relative_url
            ):
                raise JVAPIError("The response file URL is invalid.")
            parsed = urlsplit(relative_url)
            if parsed.scheme or parsed.netloc or parsed.query or parsed.fragment:
                raise JVAPIError("The response file URL must be same-origin.")
            declared_size = item.get("size_bytes")
            if (
                isinstance(declared_size, bool)
                or not isinstance(declared_size, int)
                or not 0 < declared_size <= MAX_RESPONSE_FILE_BYTES
            ):
                raise JVAPIError("The response file size is invalid.")
            total_bytes += declared_size
            if total_bytes > MAX_RESPONSE_TOTAL_BYTES:
                raise JVAPIError("The response files exceed the safe total size.")

            safe_name = _safe_download_name(item.get("name"), index)
            target = _available_download_target(destination, safe_name)
            temporary = target.with_name(f".{target.name}.part")
            try:
                with self.session.get(
                    f"{self.base_url}{relative_url}",
                    headers={"Accept": "*/*"},
                    stream=True,
                    timeout=self.request_timeout,
                ) as response:
                    if response.status_code != 200:
                        raise _safe_response_error(response)
                    content_length = response.headers.get("Content-Length")
                    if content_length is not None and (
                        not content_length.isdigit()
                        or int(content_length) != declared_size
                    ):
                        raise JVAPIError(
                            "The response file length did not match its manifest."
                        )
                    written = 0
                    with temporary.open("xb") as handle:
                        temporary.chmod(0o600)
                        for chunk in response.iter_content(chunk_size=1024 * 1024):
                            if not chunk:
                                continue
                            written += len(chunk)
                            if (
                                written > declared_size
                                or written > MAX_RESPONSE_FILE_BYTES
                            ):
                                raise JVAPIError(
                                    "The response file exceeded its safe size."
                                )
                            handle.write(chunk)
                        handle.flush()
                        os.fsync(handle.fileno())
                    if written != declared_size:
                        raise JVAPIError(
                            "The response file length did not match its manifest."
                        )
                os.link(temporary, target)
                temporary.unlink()
                target.chmod(0o600)
                downloaded.append(target)
            except Exception:
                temporary.unlink(missing_ok=True)
                raise
        return downloaded

    def logout(self) -> None:
        """Revoke the current bearer token."""
        if self._access_token is None:
            return
        try:
            response = self.session.post(
                f"{self.base_url}/v1/auth/logout",
                timeout=self.request_timeout,
            )
            _require_status(response, {204})
        except requests.RequestException as exc:
            raise JVAPIError("Could not confirm token revocation.") from exc
        finally:
            self._access_token = None
            self.session.headers.pop("Authorization", None)

    def close(self) -> None:
        self.session.close()

    def _require_login(self) -> None:
        if self._access_token is None:
            raise JVAPIError("Call login() before using an authenticated endpoint.")


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Submit one question to the public JV LLM API."
    )
    parser.add_argument("question", help="Question to submit")
    parser.add_argument(
        "--file",
        action="append",
        default=[],
        type=Path,
        dest="files",
        help="Attachment path; repeat --file for multiple files",
    )
    parser.add_argument(
        "--conversation-id",
        help="Owned conversation ID when submitting a follow-up turn",
    )
    parser.add_argument(
        "--base-url",
        default=os.environ.get("JV_API_BASE_URL", DEFAULT_BASE_URL),
        help=f"JV LLM API origin (default: {DEFAULT_BASE_URL})",
    )
    parser.add_argument(
        "--username",
        default=os.environ.get("JV_API_USERNAME", DEFAULT_USERNAME),
        help=f"JV LLM username (default: {DEFAULT_USERNAME})",
    )
    parser.add_argument(
        "--poll-interval",
        type=float,
        default=2.0,
        help="Seconds between status polls (default: 2)",
    )
    parser.add_argument(
        "--wait-timeout",
        type=float,
        default=3600.0,
        help="Maximum local wait in seconds (default: 3600)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Print the complete safe public terminal-job JSON",
    )
    parser.add_argument(
        "--download-dir",
        type=Path,
        help="Download generated response images/files into this directory",
    )
    return parser


def _safe_download_name(value: Any, index: int) -> str:
    name = Path(value).name if isinstance(value, str) else ""
    name = re.sub(r"[^A-Za-z0-9._ -]+", "_", name).strip(" .")
    if not name or name in {".", ".."}:
        name = f"jv-ai-output-{index}"
    return name[:180]


def _available_download_target(destination: Path, name: str) -> Path:
    stem = Path(name).stem or "jv-ai-output"
    suffix = Path(name).suffix
    for sequence in range(0, 1000):
        candidate_name = name if sequence == 0 else f"{stem}-{sequence}{suffix}"
        candidate = (destination / candidate_name).absolute()
        if candidate.parent != destination or candidate.is_symlink():
            continue
        if (
            not candidate.exists()
            and not candidate.with_name(f".{candidate.name}.part").exists()
        ):
            return candidate
    raise JVAPIError("Could not allocate a safe response filename.")


def _show_progress(job: dict[str, Any]) -> None:
    status = job.get("status", "unknown")
    phase = job.get("phase", "unknown")
    queue_position = job.get("queue_position")
    queue_text = (
        f", queue position {queue_position}" if isinstance(queue_position, int) else ""
    )
    print(f"Status: {status} ({phase}){queue_text}", file=sys.stderr)


def main() -> int:
    args = _build_parser().parse_args()
    password = os.environ.get("JV_API_PASSWORD")
    if password is None:
        password = getpass.getpass(f"Password for {args.username}: ")

    client = JVAIClient(args.base_url)
    logout_error: JVAPIError | None = None
    try:
        login = client.login(args.username, password)
        user = login.get("user", {})
        display_name = user.get("username") if isinstance(user, dict) else args.username
        print(f"Authenticated as {display_name}.", file=sys.stderr)

        job = client.submit_job(
            args.question,
            file_paths=args.files,
            conversation_id=args.conversation_id,
        )
        print(
            f"Created job {job['id']} in conversation "
            f"{job.get('conversation_id', 'unknown')}.",
            file=sys.stderr,
        )
        terminal = client.wait_for_job(
            job["id"],
            poll_interval=args.poll_interval,
            wait_timeout=args.wait_timeout,
            progress=_show_progress,
        )
        if args.download_dir is not None and terminal.get("status") == "succeeded":
            downloaded = client.download_response_files(
                terminal,
                args.download_dir,
            )
            for path in downloaded:
                print(f"Downloaded response file: {path}", file=sys.stderr)
        if args.json:
            print(json.dumps(terminal, ensure_ascii=False, indent=2))
        elif terminal.get("status") == "succeeded":
            answer = terminal.get("answer")
            print(answer if isinstance(answer, str) else "")
        else:
            code = terminal.get("error_code") or "JV-JOB"
            message = terminal.get("error_message") or "The JV AI job failed."
            raise JVAPIError(f"{code}: {message}")
        return 0
    except JVAPIError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("Interrupted. Any submitted server-side job continues.", file=sys.stderr)
        return 130
    finally:
        try:
            client.logout()
        except JVAPIError as exc:
            logout_error = exc
        client.close()
        if logout_error is not None:
            print(f"Warning: {logout_error}", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
