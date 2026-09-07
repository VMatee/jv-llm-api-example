from __future__ import annotations

import json
import unittest

from python.jv_api_example import JVAPIError
from python.jv_responses_example import (
    _base_request,
    _continuation,
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


if __name__ == "__main__":
    unittest.main()
