import importlib
import inspect
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Literal, Protocol, Self

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

from betterlite._endpoints.payload import CallValue

HOST = "127.0.0.1"
OK = 200
UNAUTHORIZED = 401
NOT_FOUND = 404
GOOD_KEY = "sk-test"
BATCH_ID = "batch_abc123"
MISSING_ID = "batch_missing"
INPUT_FILE_ID = "file-abc123"
BATCH_PATH = "/v1/batches"
WINDOW: Final[Literal["24h"]] = "24h"
ENDPOINT: Final[Literal["/v1/chat/completions"]] = "/v1/chat/completions"
MODEL = "gpt-4o"
DEPLOYMENT_ID = "deployment-abc"
MODEL_INFO: dict[str, JsonValue] = {"id": DEPLOYMENT_ID}
CREATED_AT = 1710000000
METADATA: dict[str, str] = {"team": "x"}

BODY: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class RecordedCall(TypedDict):
    path: str
    headers: dict[str, str]
    body: dict[str, JsonValue]


def batch_payload(
    batch_id: str, status: str, metadata: JsonValue = None
) -> dict[str, JsonValue]:
    return {
        "id": batch_id,
        "completion_window": WINDOW,
        "created_at": CREATED_AT,
        "endpoint": ENDPOINT,
        "input_file_id": INPUT_FILE_ID,
        "object": "batch",
        "status": status,
        "metadata": metadata,
        "model": MODEL,
        "request_counts": {"completed": 0, "failed": 0, "total": 1},
    }


class BatchServer(ThreadingHTTPServer):
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

    def record(self, path: str, headers: dict[str, str], body: dict[str, JsonValue]) -> None:
        self.calls.append({"path": path, "headers": headers, "body": body})

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
    owner: ClassVar["BatchServer"]

    def do_POST(self) -> None:
        declared = self.headers.get("Content-Length")
        raw = b"{}" if declared is None else self.rfile.read(int(declared))
        body = BODY.validate_json(raw)
        self.owner.record(self.path, dict(self.headers.items()), body)
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply_error(UNAUTHORIZED, "invalid api key")
            return
        if self.path != BATCH_PATH:
            self.reply_error(NOT_FOUND, f"no batch endpoint at {self.path}")
            return
        self.reply(OK, batch_payload(BATCH_ID, "validating", body.get("metadata")))

    def do_GET(self) -> None:
        self.owner.record(self.path, dict(self.headers.items()), {})
        if self.headers.get("Authorization") != f"Bearer {GOOD_KEY}":
            self.reply_error(UNAUTHORIZED, "invalid api key")
            return
        if self.path != f"{BATCH_PATH}/{BATCH_ID}":
            self.reply_error(NOT_FOUND, f"no batch at {self.path}")
            return
        self.reply(OK, batch_payload(BATCH_ID, "in_progress"))

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


class BatchDump(Protocol):
    def model_dump(self) -> dict[str, JsonValue]: ...


class BatchCall(Protocol):
    def __call__(self, *args: CallValue, **kwargs: CallValue) -> BatchDump: ...


class LiteLLMBatches(Protocol):
    def __getattr__(self, name: str) -> BatchCall: ...


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


LITELLM: LiteLLMBatches = importlib.import_module("litellm")


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows
