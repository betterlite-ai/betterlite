import json
import threading
from collections.abc import Callable
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Self
from urllib.parse import parse_qs, urlsplit

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

HOST = "127.0.0.1"
OK = 200
UNAUTHORIZED = 401
NOT_FOUND = 404
GOOD_KEY = "sk-test"
BAD_KEY = "sk-denied"
STORE_ID = "vs_files123"
FILE_ID = "file-1"
FILENAME = "doc.md"
CREATED_AT = 1700000000
USAGE_BYTES = 12
CHUNK_TEXT = "the file answers"
ATTRIBUTES: Final[dict[str, JsonValue]] = {"source": FILENAME}
STORE_PATH = f"/v1/vector_stores/{STORE_ID}/files"
FILE_PATH = f"{STORE_PATH}/{FILE_ID}"
CONTENT_PATH = f"{FILE_PATH}/content"
BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class RecordedCall(TypedDict):
    method: str
    route: str
    query: dict[str, str]
    headers: dict[str, str]
    body: dict[str, JsonValue]


def file_payload() -> dict[str, JsonValue]:
    return {
        "id": FILE_ID,
        "object": "vector_store.file",
        "created_at": CREATED_AT,
        "usage_bytes": USAGE_BYTES,
        "vector_store_id": STORE_ID,
        "status": "completed",
        "last_error": None,
        "chunking_strategy": {"type": "auto"},
        "attributes": ATTRIBUTES,
    }


def list_payload() -> dict[str, JsonValue]:
    files = [file_payload()]
    return {
        "object": "list",
        "data": files,
        "first_id": FILE_ID,
        "last_id": FILE_ID,
        "has_more": len(files) > 1,
    }


def content_payload() -> dict[str, JsonValue]:
    part: dict[str, JsonValue] = {"type": "text", "text": CHUNK_TEXT}
    return {
        "file_id": FILE_ID,
        "filename": FILENAME,
        "attributes": ATTRIBUTES,
        "content": [part],
    }


def deleted_payload() -> dict[str, JsonValue]:
    return {
        "id": FILE_ID,
        "object": "vector_store.file.deleted",
        "deleted": True,
    }


ROUTES: Final[dict[tuple[str, str], Callable[[], dict[str, JsonValue]]]] = {
    ("POST", STORE_PATH): file_payload,
    ("GET", STORE_PATH): list_payload,
    ("GET", FILE_PATH): file_payload,
    ("GET", CONTENT_PATH): content_payload,
    ("POST", FILE_PATH): file_payload,
    ("DELETE", FILE_PATH): deleted_payload,
}


def header_of(call_record: RecordedCall, name: str) -> str | None:
    lowered = name.lower()
    for header, value in call_record["headers"].items():
        if header.lower() == lowered:
            return value
    return None


class VectorFilesServer(ThreadingHTTPServer):
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

    def record(
        self,
        method: str,
        route: str,
        query: dict[str, str],
        headers: dict[str, str],
        body: dict[str, JsonValue],
    ) -> None:
        self.calls.append(
            {
                "method": method,
                "route": route,
                "query": query,
                "headers": headers,
                "body": body,
            }
        )

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
    owner: ClassVar["VectorFilesServer"]

    def do_POST(self) -> None:
        self.handle_call("POST")

    def do_GET(self) -> None:
        self.handle_call("GET")

    def do_DELETE(self) -> None:
        self.handle_call("DELETE")

    def handle_call(self, method: str) -> None:
        parsed = urlsplit(self.path)
        query = {name: values[0] for name, values in parse_qs(parsed.query).items()}
        self.owner.record(method, parsed.path, query, dict(self.headers.items()), self.read_body())
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply_error(UNAUTHORIZED, "Incorrect API key provided")
            return
        answer = ROUTES.get((method, parsed.path))
        if answer is None:
            self.send_error(NOT_FOUND)
            return
        self.reply(OK, answer())

    def read_body(self) -> dict[str, JsonValue]:
        declared = self.headers.get("Content-Length")
        raw = b"{}" if declared is None else self.rfile.read(int(declared))
        return BODY.validate_json(raw)

    def reply_error(self, status: int, message: str) -> None:
        payload: dict[str, JsonValue] = {
            "error": {
                "message": message,
                "type": "invalid_request_error",
                "param": None,
                "code": "invalid_api_key",
            },
        }
        self.reply(status, payload)

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)
