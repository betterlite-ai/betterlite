import asyncio
import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import ClassVar, TypeAlias, cast

import litellm

litellm.suppress_debug_info = True

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


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_text(value: Json, name: str) -> str | None:
    if value is None:
        return None
    return _text(value, name)


def _object(value: Json, name: str) -> dict[str, Json]:
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _number(value: Json, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def _arguments(attempt: dict[str, Json], api_base: str | None) -> dict[str, object]:
    arguments: dict[str, object] = {
        "model": _text(attempt.get("model"), "model"),
        "document": _object(attempt.get("document"), "document"),
    }
    api_key = _optional_text(attempt.get("api_key"), "api_key")
    if api_key is not None:
        arguments["api_key"] = api_key
    provider = _optional_text(attempt.get("custom_llm_provider"), "custom_llm_provider")
    if provider is not None:
        arguments["custom_llm_provider"] = provider
    extra_headers = _object(attempt.get("extra_headers"), "extra_headers")
    if extra_headers:
        arguments["extra_headers"] = extra_headers
    timeout = attempt.get("timeout_seconds")
    if timeout is not None:
        arguments["timeout"] = _number(timeout, "timeout_seconds")
    base = api_base if api_base is not None else _optional_text(attempt.get("api_base"), "api_base")
    if base is not None:
        arguments["api_base"] = base
    arguments.update(_object(attempt.get("optional_params"), "optional_params"))
    return arguments


def _status(error: Exception) -> int | None:
    carried: object = getattr(error, "status_code", None)
    return carried if isinstance(carried, int) and not isinstance(carried, bool) else None


def _decode(attempt: dict[str, Json]) -> Json:
    name = _text(attempt.get("name"), "name")
    arguments = _arguments(attempt, None)
    try:
        asyncio.run(litellm.aocr(**arguments))  # pyright: ignore[reportArgumentType]  # the case input carries the public OCR arguments
    except Exception as error:  # noqa: BLE001 - every helper exception is part of the wire contract
        return {"name": name, "ok": False, "status": _status(error)}
    return {"name": name, "ok": True}


def _call(call: dict[str, Json]) -> Json:
    server = HTTPServer(("127.0.0.1", 0), _MockMistral)
    _MockMistral.answer = json.dumps(call.get("answer")).encode()
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        base_path = _optional_text(call.get("api_base_path"), "api_base_path") or ""
        arguments = _arguments(call, f"http://127.0.0.1:{server.server_port}{base_path}")
        response = asyncio.run(litellm.aocr(**arguments))
        dumped = cast(Json, response.model_dump())
    finally:
        server.shutdown()
        thread.join()
    captured = _MockMistral.captured
    headers = _object(captured.get("headers"), "headers")
    return {
        "url_path": _text(captured.get("path"), "path"),
        "authorization": _optional_text(headers.get("authorization"), "authorization"),
        "content_type": _optional_text(headers.get("content-type"), "content-type"),
        "body": captured.get("body"),
        "response": dumped,
    }


def run(input_value: dict[str, Json]) -> Json:
    call = input_value.get("call")
    if call is not None:
        return _call(_object(call, "call"))
    attempts = input_value.get("decode")
    if not isinstance(attempts, list):
        raise ValueError("decode must be a list")
    return {"decode": [_decode(_object(attempt, "attempt")) for attempt in attempts]}
