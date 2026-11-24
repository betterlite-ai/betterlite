import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Self

from pydantic import JsonValue
from typing_extensions import TypedDict, override

HOST = "127.0.0.1"
OK = 200
UNAUTHORIZED = 401
NOT_FOUND = 404
GOOD_KEY = "sk-test"
BAD_KEY = "bad-key"
CREATED = 1700000000
JSON_TYPE = "application/json"
JOBS_PATH = "/fine_tuning/jobs"
AZURE_JOBS_PATH = "/openai/fine_tuning/jobs"
JOB_ID = "ftjob-1"
SECOND_JOB_ID = "ftjob-2"
JOB_PATH = f"{JOBS_PATH}/{JOB_ID}"
CANCEL_PATH = f"{JOB_PATH}/cancel"
AZURE_JOB_PATH = f"{AZURE_JOBS_PATH}/{JOB_ID}"
AZURE_CANCEL_PATH = f"{AZURE_JOB_PATH}/cancel"

JOB: Final[dict[str, JsonValue]] = {
    "id": JOB_ID,
    "created_at": CREATED,
    "error": None,
    "fine_tuned_model": None,
    "finished_at": None,
    "hyperparameters": {"n_epochs": 3},
    "model": "gpt-4o-mini-2024-07-18",
    "object": "fine_tuning.job",
    "organization_id": "org-1",
    "result_files": [],
    "seed": 7,
    "status": "running",
    "trained_tokens": None,
    "training_file": "file-abc",
    "validation_file": None,
    "estimated_finish": None,
    "integrations": None,
    "metadata": None,
    "method": None,
}
SECOND_JOB: Final[dict[str, JsonValue]] = {
    **JOB,
    "id": SECOND_JOB_ID,
    "status": "queued",
}
CANCELLED: Final[dict[str, JsonValue]] = {
    **JOB,
    "status": "cancelled",
}
PAGE: Final[dict[str, JsonValue]] = {
    "object": "list",
    "data": [JOB, SECOND_JOB],
    "has_more": False,
    "first_id": JOB_ID,
    "last_id": SECOND_JOB_ID,
}
UNAUTHORIZED_BODY: Final[dict[str, JsonValue]] = {
    "error": {
        "message": "Invalid API key provided: bad-key",
        "type": "invalid_request_error",
        "param": None,
        "code": "invalid_api_key",
    }
}

# One fixed answer per route, shaped as the family that calls it reads it.
ROUTES: Final[dict[tuple[str, str], dict[str, JsonValue]]] = {
    ("POST", JOBS_PATH): JOB,
    ("POST", AZURE_JOBS_PATH): JOB,
    ("GET", JOBS_PATH): PAGE,
    ("GET", AZURE_JOBS_PATH): PAGE,
    ("GET", JOB_PATH): JOB,
    ("GET", AZURE_JOB_PATH): JOB,
    ("POST", CANCEL_PATH): CANCELLED,
    ("POST", AZURE_CANCEL_PATH): CANCELLED,
}


class RecordedCall(TypedDict):
    method: str
    path: str
    query: str
    headers: dict[str, str]
    body: bytes


class TuningServer(ThreadingHTTPServer):
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
        return f"http://{HOST}:{self.server_port}"

    def record(self, method: str, target: str, headers: dict[str, str], body: bytes) -> None:
        path, _, query = target.partition("?")
        lowered = {name.lower(): value for name, value in headers.items()}
        self.calls.append(
            {"method": method, "path": path, "query": query, "headers": lowered, "body": body}
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
    owner: ClassVar[TuningServer]

    def do_GET(self) -> None:
        self.handle_call()

    def do_POST(self) -> None:
        self.handle_call()

    def handle_call(self) -> None:
        declared = self.headers.get("Content-Length")
        body = b"" if declared is None else self.rfile.read(int(declared))
        self.owner.record(self.command, self.path, dict(self.headers.items()), body)
        key = self.headers.get("api-key")
        authorization = self.headers.get("Authorization")
        if key == BAD_KEY or authorization == f"Bearer {BAD_KEY}":
            self.reply(UNAUTHORIZED, json.dumps(UNAUTHORIZED_BODY).encode())
            return
        path, _, _query = self.path.partition("?")
        answer = ROUTES.get((self.command, path))
        if answer is None:
            self.send_error(NOT_FOUND)
            return
        self.reply(OK, json.dumps(answer).encode())

    def reply(self, status: int, content: bytes) -> None:
        self.send_response(status)
        self.send_header("Content-Type", JSON_TYPE)
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)
