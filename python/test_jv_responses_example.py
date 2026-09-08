from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from PIL import Image

from python.jv_api_example import JVAPIError
from python.attachments import image_part
from python.jv_responses_example import (
    _base_request,
    _continuation,
    _custom_continuation,
    _custom_tool_call,
    _custom_tool_request,
    _image_tool_continuation,
    _text_output,
    _tool_call,
    _tool_request,
    _validate_response,
)


def response(status: str, output: list[object]) -> dict[str, object]:
    return {
        "id": "response-1",
        "object": "response",
        "status": status,
        "output": output,
        "error": None,
    }


class ResponsesContractTest(unittest.TestCase):
    def test_text_request_and_output(self) -> None:
        request = _base_request("hello")
        self.assertEqual(request["model"], "jv-ai")
        self.assertTrue(request["background"])
        self.assertFalse(request["stream"])
        self.assertEqual(request["tool_choice"], "none")
        value = response(
            "completed",
            [
                {
                    "type": "message",
                    "id": "msg-1",
                    "status": "completed",
                    "role": "assistant",
                    "content": [
                        {"type": "output_text", "text": "hello", "annotations": []}
                    ],
                }
            ],
        )
        self.assertEqual(_text_output(value), "hello")

    def test_tool_call_and_continuation(self) -> None:
        request = _tool_request("check")
        self.assertEqual(request["tool_choice"], "required")
        self.assertEqual(request["tools"][0]["name"], "get_client_platform")
        call = response(
            "completed",
            [
                {
                    "type": "function_call",
                    "id": "fc-1",
                    "call_id": "call-1",
                    "status": "completed",
                    "name": "get_client_platform",
                    "arguments": "{}",
                }
            ],
        )
        call_id, result = _tool_call(call)
        self.assertEqual(call_id, "call-1")
        self.assertTrue(result)
        continuation = _continuation("response-1", call_id, result)
        self.assertEqual(continuation["previous_response_id"], "response-1")
        self.assertEqual(continuation["input"][0]["type"], "function_call_output")
        self.assertEqual(continuation["input"][0]["call_id"], "call-1")

    def test_malformed_output_fails_closed(self) -> None:
        cases = [
            response("queued", [{"type": "message"}]),
            response("completed", []),
            response("completed", [{"type": "message"}, {"type": "message"}]),
            response(
                "completed",
                [
                    {
                        "type": "function_call",
                        "id": "fc-1",
                        "call_id": "call-1",
                        "status": "completed",
                        "name": "get_client_platform",
                        "arguments": json.dumps({"unexpected": True}),
                    }
                ],
            ),
        ]
        for value in cases[:3]:
            with self.assertRaises(JVAPIError):
                _validate_response(value)
        with self.assertRaises(JVAPIError):
            _tool_call(cases[3])

    def test_image_bearing_function_result(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "view.png"
            Image.new("RGB", (4, 4), "blue").save(path)
            part = image_part(path, "high")
        request = _image_tool_continuation("response-1", "call-1", part)
        result = request["input"][0]
        self.assertEqual(result["type"], "function_call_output")
        self.assertEqual(result["call_id"], "call-1")
        self.assertEqual([item["type"] for item in result["output"]], ["input_image"])
        self.assertEqual(result["output"][0]["detail"], "high")
        self.assertTrue(result["output"][0]["image_url"].startswith("data:image/png;base64,"))

    def test_pinned_custom_apply_patch_shapes(self) -> None:
        grammar = Path("examples/codex-0.149.1-apply-patch.lark").read_text()
        declaration = _custom_tool_request("Edit the fixture", grammar)
        self.assertEqual(declaration["tools"][0]["type"], "custom")
        self.assertEqual(declaration["tools"][0]["name"], "apply_patch")
        self.assertEqual(declaration["tools"][0]["format"]["syntax"], "lark")
        freeform = "*** Begin Patch\n*** Add File: example.txt\n+example\n*** End Patch"
        call = response(
            "completed",
            [{"type": "custom_tool_call", "id": "ctc-1", "call_id": "call-1", "name": "apply_patch", "input": freeform}],
        )
        call_id, actual = _custom_tool_call(call)
        self.assertEqual((call_id, actual), ("call-1", freeform))
        result = _custom_continuation("response-1", call_id, "Success. Updated example.txt")
        self.assertEqual(result["previous_response_id"], "response-1")
        self.assertEqual(result["input"], [{"type": "custom_tool_call_output", "call_id": "call-1", "output": "Success. Updated example.txt"}])

    def test_custom_subset_rejects_other_grammar_and_call(self) -> None:
        with self.assertRaises(JVAPIError):
            _custom_tool_request("edit", "start: anything")
        invalid = response(
            "completed",
            [{"type": "custom_tool_call", "id": "ctc-1", "call_id": "call-1", "name": "other", "input": "opaque"}],
        )
        with self.assertRaises(JVAPIError):
            _custom_tool_call(invalid)


if __name__ == "__main__":
    unittest.main()
