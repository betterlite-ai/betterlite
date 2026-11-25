import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Self

from typing_extensions import TypedDict, override

HOST = "127.0.0.1"
OK = 200
UNAUTHORIZED = 401
NOT_FOUND = 404
GOOD_KEY = "sk-test"
FILE_ID = "file-abc123"
JSON_ID = "file-json"
MISSING_ID = "file-missing"
BODY: Final = b"file body"
JSON_BODY: Final = b'{"ok":true}'
CONTENT_TYPE: Final = "application/octet-stream"
JSON_TYPE: Final = "application/json"
UPSTREAM_HEADER: Final = "x-upstream"
UPSTREAM_VALUE: Final = "yes"
CONTENT_PATH: Final = f"/v1/files/{FILE_ID}/content"
JSON_PATH: Final = f"/v1/files/{JSON_ID}/content"
BODIES: Final[dict[str, tuple[str, bytes]]] = {
    CONTENT_PATH: (CONTENT_TYPE, BODY),
    JSON_PATH: (JSON_TYPE, JSON_BODY),
}


class RecordedCall(TypedDict):
    method: str
    path: str
    headers: dict[str, str]


class FileServer(ThreadingHTTPServer):
    daemon_threads = True
    calls: list[RecordedCall]

    def __init__(self) -> None:
        super().__init__((HOST, 0), _Handler)
        self.calls = []
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}/v1"

    def record(self, method: str, path: str, headers: dict[str, str]) -> None:
        self.calls.append({"method": method, "path": path, "headers": headers})

    @override
    def __enter__(self) -> Self:
        return self

    @override
    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class _Handler(BaseHTTPRequestHandler):
    owner: ClassVar[FileServer]
    protocol_version = "HTTP/1.1"

    def do_GET(self) -> None:
        self.owner.record(self.command, self.path, dict(self.headers.items()))
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply_error(UNAUTHORIZED, "invalid api key")
            return
        served = BODIES.get(self.path)
        if served is None:
            self.reply_error(NOT_FOUND, "no such file")
            return
        content_type, payload = served
        self.send_response(OK)
        self.send_header("Content-Type", content_type)
        self.send_header(UPSTREAM_HEADER, UPSTREAM_VALUE)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def reply_error(self, status: int, message: str) -> None:
        payload = {
            "error": {
                "message": message,
                "type": "invalid_request_error",
                "code": "file_not_found",
            },
        }
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)
