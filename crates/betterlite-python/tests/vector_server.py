import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Self

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

HOST = "127.0.0.1"
OK = 200
UNAUTHORIZED = 401
NOT_FOUND = 404
GOOD_KEY = "sk-test"
BAD_KEY = "sk-denied"
STORE_ID = "vs_test123"
STORE_NAME = "documents"
STORE_PATH = "/v1/vector_stores"
SEARCH_PATH = f"/v1/vector_stores/{STORE_ID}/search"
SLOW_STORE_ID = "vs_slow123"
SLOW_PATH = f"/v1/vector_stores/{SLOW_STORE_ID}/search"
SLOW_SECONDS = 2.0
CREATED_AT = 1700000000
EXPIRES_DAYS = 7
DAY_SECONDS = 86400
SEARCH_QUERY = "sample query"
RESULT_TEXT = "the vector store answers"
RESULT_SCORE = 0.95
FILE_ID = "file-1"
FILENAME = "doc.md"
METADATA: Final[dict[str, JsonValue]] = {"team": "search"}
BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class RecordedCall(TypedDict):
    method: str
    path: str
    headers: dict[str, str]
    body: dict[str, JsonValue]


def create_payload() -> dict[str, JsonValue]:
    counts: dict[str, JsonValue] = {
        "in_progress": 0,
        "completed": 0,
        "failed": 0,
        "cancelled": 0,
        "total": 0,
    }
    return {
        "id": STORE_ID,
        "object": "vector_store",
        "created_at": CREATED_AT,
        "name": STORE_NAME,
        "bytes": 0,
        "file_counts": counts,
        "status": "completed",
        "expires_after": {"anchor": "last_active_at", "days": EXPIRES_DAYS},
        "expires_at": CREATED_AT + EXPIRES_DAYS * DAY_SECONDS,
        "last_active_at": CREATED_AT,
        "metadata": METADATA,
    }


def search_payload() -> dict[str, JsonValue]:
    content: dict[str, JsonValue] = {"text": RESULT_TEXT, "type": "text"}
    result: dict[str, JsonValue] = {
        "score": RESULT_SCORE,
        "content": [content],
        "file_id": FILE_ID,
        "filename": FILENAME,
        "attributes": {"source": FILENAME},
    }
    return {
        "object": "vector_store.search_results.page",
        "search_query": SEARCH_QUERY,
        "data": [result],
    }


def header_of(call_record: RecordedCall, name: str) -> str | None:
    lowered = name.lower()
    for header, value in call_record["headers"].items():
        if header.lower() == lowered:
            return value
    return None


class VectorServer(ThreadingHTTPServer):
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
        self, method: str, path: str, headers: dict[str, str], body: dict[str, JsonValue]
    ) -> None:
        self.calls.append(
            {"method": method, "path": path, "headers": headers, "body": body}
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
    owner: ClassVar["VectorServer"]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        raw = b"{}" if declared is None else self.rfile.read(int(declared))
        body = BODY.validate_json(raw)
        self.owner.record("POST", self.path, dict(self.headers.items()), body)
        authorized = self.headers.get("Authorization")
        if authorized != f"Bearer {GOOD_KEY}":
            self.reply_error(UNAUTHORIZED, "Incorrect API key provided")
            return
        if self.path == STORE_PATH:
            self.reply(OK, create_payload())
            return
        if self.path == SEARCH_PATH:
            self.reply(OK, search_payload())
            return
        if self.path == SLOW_PATH:
            # the caller leaves at its own timeout; answering this late would only break the pipe
            time.sleep(SLOW_SECONDS)
            self.close_connection = True
            return
        self.send_error(NOT_FOUND)

    def reply_error(self, status: int, message: str) -> None:
        payload: dict[str, JsonValue] = {
            "error": {"message": message, "type": "invalid_request_error"},
        }
        self.reply(status, payload)

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)
