import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import TypeAlias, cast

import litellm
from litellm.llms.base_llm.ocr.transformation import OCRResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _Capture:
    def __init__(self) -> None:
        self.path = ""
        self.authorization: str | None = None
        self.content_type: str | None = None
        self.body = b""


class _Server(ThreadingHTTPServer):
    def __init__(self, reply: bytes) -> None:
        super().__init__(("127.0.0.1", 0), _Handler)
        self.reply = reply
        self.capture = _Capture()


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_POST(self) -> None:
        server = cast(_Server, self.server)
        length = int(self.headers.get("content-length", "0"))
        server.capture.path = self.path
        server.capture.authorization = self.headers.get("authorization")
        server.capture.content_type = self.headers.get("content-type")
        server.capture.body = self.rfile.read(length)
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(server.reply)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(server.reply)

    def log_message(self, format: str, *args: object) -> None:  # noqa: A002
        del format, args


def _decode(payload: bytes) -> Json:
    return cast(Json, json.loads(payload))


def _string(input_value: dict[str, Json], name: str) -> str:
    value = input_value.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _object(input_value: dict[str, Json], name: str) -> dict[str, Json]:
    value = input_value.get(name)
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast("dict[str, Json]", value)


def _optional_string(input_value: dict[str, Json], name: str) -> str | None:
    value = input_value.get(name)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string or null")
    return value


def _optional_object(input_value: dict[str, Json], name: str) -> dict[str, Json] | None:
    value = input_value.get(name)
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object or null")
    return cast("dict[str, Json]", value)


def _start_server(reply: bytes) -> tuple[_Server, threading.Thread]:
    server = _Server(reply)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, thread


def run(input_value: dict[str, Json]) -> Json:
    model = _string(input_value, "model")
    document = _object(input_value, "document")
    response = _object(input_value, "response")
    options = _optional_object(input_value, "options")
    api_key = _optional_string(input_value, "api_key")
    extra_headers = _optional_object(input_value, "extra_headers")
    server, thread = _start_server(json.dumps(response).encode())
    try:
        port = cast("tuple[str, int]", server.server_address)[1]
        answer = cast(
            OCRResponse,
            litellm.ocr(
                model=f"cohere/{model}",
                document=document,
                api_key=api_key,
                api_base=f"http://127.0.0.1:{port}",
                extra_headers=extra_headers,
                **(options or {}),
            ),
        )
    finally:
        server.shutdown()
        thread.join()
    capture = server.capture
    return {
        "request": {
            "method": "POST",
            "path": capture.path,
            "authorization": capture.authorization,
            "content_type": capture.content_type,
            "body": _decode(capture.body),
        },
        "response": cast(Json, answer.model_dump(mode="json")),
        "native": cast(Json, answer.get_provider_native_response()),
    }
