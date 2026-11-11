import asyncio
import gzip
import hashlib
import os
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.proxy.middleware.gzip_middleware import GZipBufferedResponseMiddleware
from litellm.proxy.middleware.security_headers_middleware import SecurityHeadersMiddleware
from starlette.types import Message, Receive, Scope, Send

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    function: Literal["gzip", "security_headers"]
    accept_encoding: NotRequired[str]
    status: NotRequired[int]
    headers: NotRequired[dict[str, str]]
    body_size: NotRequired[int]
    body_byte: NotRequired[int]
    enable_hsts: NotRequired[bool]


class AsgiMessage(TypedDict, total=False):
    type: str
    status: int
    headers: list[tuple[bytes, bytes]]
    body: bytes


class Recorder:
    def __init__(self) -> None:
        self.status: int | None = None
        self.headers: dict[str, str] = {}
        self.body = b""

    async def __call__(self, message: Message) -> None:
        typed = cast(AsgiMessage, message)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/middleware/gzip_middleware.py#L8
        if typed.get("type") == "http.response.start":
            status = typed.get("status")
            headers = typed.get("headers")
            if status is not None:
                self.status = status
            if headers is not None:
                self.headers = {
                    name.decode("latin-1").lower(): value.decode("latin-1")
                    for name, value in headers
                }
            return
        body = typed.get("body")
        if body is not None:
            self.body += body


class ResponseApp:
    def __init__(self, status: int, headers: dict[str, str], body: bytes) -> None:
        self.status = status
        self.headers = headers
        self.body = body

    async def __call__(self, _scope: Scope, _receive: Receive, send: Send) -> None:
        headers = [
            (name.lower().encode("latin-1"), value.encode("latin-1"))
            for name, value in self.headers.items()
        ]
        start: Message = {"type": "http.response.start", "status": self.status, "headers": headers}
        await send(start)
        body: Message = {"type": "http.response.body", "body": self.body}
        await send(body)


async def _receive() -> Message:
    request: Message = {"type": "http.request"}
    return request


def _header_scope(accept_encoding: str | None) -> Scope:
    headers: list[tuple[bytes, bytes]] = []
    if accept_encoding is not None:
        headers.append((b"accept-encoding", accept_encoding.encode("latin-1")))
    scope: Scope = {"type": "http", "method": "GET", "path": "/", "headers": headers}
    return scope


def _requested_headers(input_value: Input) -> dict[str, str]:
    configured = input_value.get("headers")
    if configured is None:
        return {}
    return dict(configured)


def _requested_status(input_value: Input) -> int:
    status = input_value.get("status")
    if status is None:
        return 200
    return status


def _body_bytes(input_value: Input) -> bytes:
    size = input_value.get("body_size")
    if size is None:
        raise ValueError("gzip needs body_size")
    byte = input_value.get("body_byte")
    if byte is None:
        byte = 97
    return bytes([byte]) * size


async def _gzip(input_value: Input) -> Json:
    body = _body_bytes(input_value)
    headers = _requested_headers(input_value)
    headers["content-length"] = str(len(body))
    recorder = Recorder()
    app = ResponseApp(_requested_status(input_value), headers, body)
    scope = _header_scope(input_value.get("accept_encoding"))
    middleware = GZipBufferedResponseMiddleware(app)
    await middleware(scope, _receive, recorder)
    if recorder.status is None:
        raise ValueError("the gzip middleware sent no response start")
    encoding = recorder.headers.get("content-encoding")
    decoded = gzip.decompress(recorder.body) if encoding == "gzip" else recorder.body
    return {
        "status": recorder.status,
        "compressed": encoding == "gzip",
        "content_encoding": encoding,
        "vary": recorder.headers.get("vary"),
        "content_length_matches": recorder.headers.get("content-length") == str(len(recorder.body)),
        "decoded_sha256": hashlib.sha256(decoded).hexdigest(),
        "decoded_length": len(decoded),
    }


async def _security_headers(input_value: Input) -> Json:
    enabled = input_value.get("enable_hsts")
    os.environ["LITELLM_ENABLE_HSTS"] = "true" if enabled else "false"
    recorder = Recorder()
    app = ResponseApp(_requested_status(input_value), _requested_headers(input_value), b"")
    await SecurityHeadersMiddleware(app)(_header_scope(None), _receive, recorder)
    if recorder.status is None:
        raise ValueError("the security headers middleware sent no response start")
    return {
        "status": recorder.status,
        "x-frame-options": recorder.headers.get("x-frame-options"),
        "content-security-policy": recorder.headers.get("content-security-policy"),
        "x-content-type-options": recorder.headers.get("x-content-type-options"),
        "strict-transport-security": recorder.headers.get("strict-transport-security"),
    }


def run(input_value: Input) -> Json:
    if input_value["function"] == "gzip":
        return asyncio.run(_gzip(input_value))
    return asyncio.run(_security_headers(input_value))
