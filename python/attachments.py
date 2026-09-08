"""Bounded local attachment helpers; server admission remains authoritative."""

import base64
import io
import json
import os
import stat
from pathlib import Path

from PIL import Image

FILE_LIMIT = 10 * 1024 * 1024
MIMES = {
    ".txt": "text/plain",
    ".md": "text/markdown",
    ".pdf": "application/pdf",
    ".json": "application/json",
    ".csv": "text/csv",
    ".py": "text/x-python",
    ".docx": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ".xlsx": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ".pptx": "application/vnd.openxmlformats-officedocument.presentationml.presentation",
}


def read_local(path, limit):
    path = Path(path)
    if not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError("Attachment must be a regular, non-symlink local file")
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    with os.fdopen(descriptor, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode) or not 0 < metadata.st_size <= limit:
            raise ValueError("Attachment is empty or oversized")
        content = stream.read(limit + 1)
    if not 0 < len(content) <= limit:
        raise ValueError("Attachment is empty or oversized")
    return content


def local_file(path):
    path = Path(path)
    mime = MIMES.get(path.suffix.lower())
    if mime is None:
        raise ValueError("Unsupported structured file extension")
    data = read_local(path, FILE_LIMIT)
    if mime.startswith("text/") or mime == "application/json":
        text = data.decode("utf-8", "strict")
        if "\0" in text:
            raise ValueError("NUL in text attachment")
        if mime == "application/json":
            json.loads(text)
    elif mime == "application/pdf":
        if not data.startswith(b"%PDF-") or b"%%EOF" not in data[-2048:]:
            raise ValueError("Invalid PDF signature")
    elif not data.startswith(b"PK\x03\x04"):
        raise ValueError("Invalid Office container signature")
    # This is only preflight: central validates the actual complete container.
    name = "".join(
        c if c.isascii() and (c.isalnum() or c in "._-") else "_" for c in path.name
    )
    suffix = path.suffix.lower()
    name = name[: -len(suffix)][: 160 - len(suffix)] + suffix
    return name, mime, data


def image_part(path, detail="auto"):
    if detail not in {"auto", "high"}:
        raise ValueError("Image detail must be auto or high")
    data = read_local(path, 5 * 1024 * 1024)
    with Image.open(io.BytesIO(data), formats=["PNG", "JPEG", "WEBP"]) as image:
        if (
            image.width * image.height > 16_000_000
            or max(image.size) > 8192
            or getattr(image, "n_frames", 1) != 1
        ):
            raise ValueError("Unsupported image dimensions or animation")
        mime = {"PNG": "image/png", "JPEG": "image/jpeg", "WEBP": "image/webp"}[
            image.format
        ]
        image.verify()
    with Image.open(io.BytesIO(data)) as image:
        image.load()
    return {
        "type": "input_image",
        "image_url": f"data:{mime};base64," + base64.b64encode(data).decode("ascii"),
        "detail": detail,
    }
