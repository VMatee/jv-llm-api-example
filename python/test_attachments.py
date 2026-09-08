import argparse
import json
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from PIL import Image
from python.attachments import image_part, local_file
from python.jv_responses_example import (
    AttachmentAction,
    JVResponsesClient,
    _base_request,
    _continuation,
)


class AttachmentsTest(unittest.TestCase):
    def test_local_formats_and_boundaries(self):
        with tempfile.TemporaryDirectory() as folder:
            for suffix, data, mime in [
                ("txt", b"fact", "text/plain"),
                ("md", b"# fact", "text/markdown"),
                ("json", b"{}", "application/json"),
                ("csv", b"a,b\n1,2", "text/csv"),
                ("py", b"X=1", "text/x-python"),
            ]:
                path = Path(folder) / ("fact." + suffix)
                path.write_bytes(data)
                self.assertEqual(local_file(path)[1:], (mime, data))
            path = Path(folder) / "bad.docx"
            path.write_bytes(b"bad")
            with self.assertRaises(ValueError):
                local_file(path)
            for fmt, suffix in [("PNG", "png"), ("JPEG", "jpg"), ("WEBP", "webp")]:
                path = Path(folder) / ("image." + suffix)
                Image.new("RGB", (4, 4), "red").save(path, fmt)
                self.assertEqual(image_part(path, "high")["detail"], "high")
            with self.assertRaises(ValueError):
                image_part(path, "low")
            path = Path(folder) / "empty.txt"
            path.touch()
            with self.assertRaises(ValueError):
                local_file(path)

    def test_option_order(self):
        parser = argparse.ArgumentParser()
        parser.add_argument(
            "--file", "--image", dest="attachments", action=AttachmentAction
        )
        args = parser.parse_args(
            ["--file", "a.txt", "--image", "b.png", "--file", "c.pdf"]
        )
        self.assertEqual(
            args.attachments,
            [("--file", "a.txt"), ("--image", "b.png"), ("--file", "c.pdf")],
        )

    def test_http_staging_mixed_and_continuation(self):
        requests = []
        file_id = "file_" + "A" * 43

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                requests.append((self.path, dict(self.headers), body))
                if self.path.endswith("login"):
                    value = {"access_token": "offline-token"}
                    status = 200
                elif self.path == "/v1/files":
                    value = {
                        "id": file_id,
                        "object": "file",
                        "bytes": 2,
                        "filename": "facts.json",
                        "media_type": "application/json",
                        "created_at": 1,
                        "expires_at": 7201,
                    }
                    status = 201
                elif self.path.endswith("logout"):
                    self.send_response(204)
                    self.end_headers()
                    return
                else:
                    value = {
                        "id": "r1",
                        "object": "response",
                        "status": "queued",
                        "output": [],
                        "error": None,
                    }
                    status = 202
                data = json.dumps(value).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                requests.append((self.path, dict(self.headers), b""))
                data = json.dumps(
                    {
                        "id": "r1",
                        "object": "response",
                        "status": "completed",
                        "error": None,
                        "output": [
                            {
                                "type": "message",
                                "role": "assistant",
                                "status": "completed",
                                "content": [{"type": "output_text", "text": "done"}],
                            }
                        ],
                    }
                ).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        client = JVResponsesClient(f"http://127.0.0.1:{server.server_port}")
        try:
            with tempfile.TemporaryDirectory() as folder:
                path = Path(folder) / "facts.json"
                path.write_bytes(b"{}")
                picture = Path(folder) / "scene.png"
                Image.new("RGB", (4, 4), "red").save(picture)
                client.login("fixture", "offline")
                staged = client.stage_file(path, "upload-1")
                body = _base_request("read")
                body["input"][0]["content"] = [
                    {"type": "input_file", "file_id": staged["id"]},
                    image_part(picture),
                ]
                client.create(body, "round-1")
                client.wait("r1", 0.01, 1)
                client.create(_continuation("r1", "call1", "ok"), "round-2")
                client.logout()
            self.assertIn(b'filename="facts.json"', requests[1][2])
            self.assertEqual(requests[1][1]["Idempotency-Key"], "upload-1")
            self.assertEqual(requests[1][1]["X-JV-CSRF"], "1")
            self.assertEqual(
                json.loads(requests[2][2])["input"][0]["content"][0]["file_id"], file_id
            )
            self.assertEqual(requests[3][0], "/v1/responses/r1")
            self.assertNotIn(b"file_id", requests[4][2])
        finally:
            client.close()
            server.shutdown()
            server.server_close()
            thread.join()
