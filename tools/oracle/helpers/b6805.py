import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import TypeAlias, cast

import litellm
from litellm.llms.base_llm.ocr.transformation import OCRResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _Reply:
    def __init__(self, status: int, headers: list[tuple[str, str]], body: bytes) -> None:
        self.status = status
        self.headers = headers
        self.body = body


class _Capture:
    def __init__(self) -> None:
        self.requests: list[dict[str, Json]] = []


class _Server(ThreadingHTTPServer):
    def __init__(self, replies: list[_Reply]) -> None:
        super().__init__(("127.0.0.1", 0), _Handler)
        self.replies = replies
        self.capture = _Capture()
        self.count = 0
        self.port = cast("tuple[str, int]", self.server_address)[1]

    def next_reply(self) -> _Reply:
        if not self.replies:
            return _Reply(500, [], b"")
        reply = self.replies[min(self.count, len(self.replies) - 1)]
        self.count += 1
        return reply


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _serve(self) -> None:
        server = cast(_Server, self.server)
        length = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(length) if length else b""
        server.capture.requests.append(
            {
                "method": self.command,
                "path": self.path,
                "authorization": self.headers.get("authorization"),
                "subscription_key": self.headers.get("ocp-apim-subscription-key"),
                "content_type": self.headers.get("content-type"),
                "body": _decode(body) if body else None,
            }
        )
        reply = server.next_reply()
        self.send_response(reply.status)
        for name, value in reply.headers:
            resolved = f"http://127.0.0.1:{server.port}{value}" if value.startswith("/") else value
            self.send_header(name, resolved)
        self.send_header("content-length", str(len(reply.body)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(reply.body)

    def do_POST(self) -> None:
        self._serve()

    def do_GET(self) -> None:
        self._serve()

    def log_message(self, format: str, *args: object) -> None:  # noqa: A002
        del format, args


def _decode(payload: bytes) -> Json:
    return cast(Json, json.loads(payload))


def _string(input_value: dict[str, Json], name: str) -> str:
    value = input_value.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _headers(value: Json) -> list[tuple[str, str]]:
    if value is None:
        return []
    if not isinstance(value, dict):
        raise ValueError("reply headers must be an object")
    listed: list[tuple[str, str]] = []
    for name, header in value.items():
        if not isinstance(header, str):
            raise ValueError(f"the reply header {name} is not a string")
        listed.append((name, header))
    return listed


def _reply(value: Json) -> _Reply:
    if not isinstance(value, dict):
        raise ValueError("a reply must be an object")
    status = value.get("status")
    if not isinstance(status, int) or isinstance(status, bool):
        raise ValueError("a reply status must be an integer")
    body = value.get("body")
    encoded = b"" if body is None else json.dumps(body).encode()
    return _Reply(status, _headers(value.get("headers")), encoded)


def _replies(value: Json) -> list[_Reply]:
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError("polls must be a list")
    return [_reply(item) for item in value]


def run(input_value: dict[str, Json]) -> Json:
    model = _string(input_value, "model")
    document = cast("dict[str, Json]", input_value.get("document"))
    analyze = _reply(input_value.get("analyze"))
    polls = _replies(input_value.get("polls"))
    options = cast("dict[str, Json]", input_value.get("options") or {})
    api_key = cast("str | None", input_value.get("api_key"))
    extra_headers = cast("dict[str, Json] | None", input_value.get("extra_headers"))
    server = _Server([analyze, *polls])
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        answer = cast(
            OCRResponse,
            litellm.ocr(
                model=f"azure_ai/{model}",
                document=document,
                api_key=api_key,
                api_base=f"http://127.0.0.1:{server.port}",
                extra_headers=extra_headers,
                **options,
            ),
        )
    finally:
        server.shutdown()
        thread.join()
    return {
        "requests": cast(Json, server.capture.requests),
        "response": cast(Json, answer.model_dump(mode="json")),
        "native": cast(Json, answer.get_provider_native_response()),
    }
