import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import ClassVar, TypeAlias, cast

import litellm
from litellm.llms.base_llm.ocr.transformation import OCRResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_HEADERS = ("authorization", "content-type")


class _MockMistral(BaseHTTPRequestHandler):
    answer: ClassVar[bytes] = b""
    captured: ClassVar[dict[str, Json]] = {}

    def do_POST(self) -> None:
        length = int(self.headers.get("content-length", "0"))
        raw_body = self.rfile.read(length)
        headers: dict[str, Json] = {}
        for name in _HEADERS:
            value = self.headers.get(name)
            if value is not None:
                headers[name] = value
        type(self).captured = {
            "path": self.path,
            "headers": headers,
            "body": cast(Json, json.loads(raw_body)),
        }
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(self.answer)))
        self.end_headers()
        self.wfile.write(self.answer)


def _required(input_value: dict[str, Json], name: str) -> Json:
    if name not in input_value:
        raise ValueError(f"{name} is required")
    return input_value[name]


def _string(input_value: dict[str, Json], name: str) -> str:
    value = input_value.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_string(input_value: dict[str, Json], name: str) -> str | None:
    value = input_value.get(name)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _object(input_value: dict[str, Json], name: str) -> dict[str, Json]:
    value = input_value.get(name)
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _call_arguments(input_value: dict[str, Json], port: int) -> dict[str, object]:
    base_path = _optional_string(input_value, "api_base_path") or ""
    base_query = _optional_string(input_value, "api_base_query") or ""
    arguments: dict[str, object] = {
        "model": f"mistral/{_string(input_value, 'model')}",
        "document": _object(input_value, "document"),
        "api_base": f"http://127.0.0.1:{port}{base_path}{base_query}",
    }
    api_key = _optional_string(input_value, "api_key")
    if api_key is not None:
        arguments["api_key"] = api_key
    extra_headers = _object(input_value, "extra_headers")
    if extra_headers:
        arguments["extra_headers"] = extra_headers
    arguments.update(_object(input_value, "options"))
    return arguments


def run(input_value: dict[str, Json]) -> Json:
    server = HTTPServer(("127.0.0.1", 0), _MockMistral)
    _MockMistral.answer = json.dumps(_required(input_value, "answer")).encode()
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        arguments = _call_arguments(input_value, server.server_port)
        response = cast(OCRResponse, litellm.ocr(**arguments))
        dumped = cast(Json, response.model_dump())
    finally:
        server.shutdown()
        thread.join()
    captured = _MockMistral.captured
    headers = _object(captured, "headers")
    return {
        "url_path": _string(captured, "path"),
        "authorization": _optional_string(headers, "authorization"),
        "content_type": _optional_string(headers, "content-type"),
        "body": _required(captured, "body"),
        "response": dumped,
    }
