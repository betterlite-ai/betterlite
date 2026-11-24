import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, NotRequired, Self

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

HOST = "127.0.0.1"
OK = 200
UNAUTHORIZED = 401
NOT_FOUND = 404
RATE_LIMITED = 429
SERVER_ERROR = 500
GOOD_KEY = "sk-test"
BAD_KEY = "sk-denied"
RATE_LIMITED_KEY = "sk-rate-limited"
FAILING_KEY = "sk-failing"
ASSISTANT_ID = "asst_1"
THREAD_ID = "thread_1"
CREATED = 1700000000
ASSISTANTS_PATH = "/v1/assistants"
THREADS_PATH = "/v1/threads"
AUTHORIZATION: Final = "Authorization"
BEARER: Final = "Bearer "

ASSISTANT: dict[str, JsonValue] = {
    "id": ASSISTANT_ID,
    "object": "assistant",
    "created_at": CREATED,
    "name": "helper",
    "description": "a helper",
    "instructions": "be brief",
    "model": "gpt-4o-mini",
    "tools": [{"type": "code_interpreter"}],
    "tool_resources": {
        "code_interpreter": {"file_ids": ["file_1"]},
        "file_search": {"vector_store_ids": ["vs_1"]},
    },
    "metadata": {"team": "a"},
    "temperature": 0.2,
    "top_p": 0.9,
    "response_format": "auto",
}
PAGE: dict[str, JsonValue] = {
    "object": "list",
    "data": [ASSISTANT],
    "first_id": ASSISTANT_ID,
    "last_id": ASSISTANT_ID,
    "has_more": False,
}
DELETED: dict[str, JsonValue] = {
    "id": ASSISTANT_ID,
    "object": "assistant.deleted",
    "deleted": True,
}
THREAD: dict[str, JsonValue] = {
    "id": THREAD_ID,
    "object": "thread",
    "created_at": CREATED,
    "metadata": {"team": "a"},
}
RATE_LIMITED_BODY: dict[str, JsonValue] = {
    "error": {
        "message": "Rate limit reached for requests",
        "type": "requests",
        "param": None,
        "code": "rate_limit_exceeded",
    },
}
SERVER_ERROR_BODY: dict[str, JsonValue] = {
    "error": {
        "message": "The server had an error while processing your request",
        "type": "server_error",
        "param": None,
        "code": None,
    },
}


class RecordedCall(TypedDict):
    method: str
    path: str
    headers: dict[str, str]
    body: NotRequired[JsonValue]


class AssistantServer(ThreadingHTTPServer):
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

    def record(self, call: RecordedCall) -> None:
        self.calls.append(call)

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
    owner: ClassVar[AssistantServer]
    POST_ROUTES: ClassVar[dict[str, dict[str, JsonValue]]] = {
        ASSISTANTS_PATH: ASSISTANT,
        THREADS_PATH: THREAD,
    }

    def do_GET(self) -> None:
        self.record({})
        if not self.authorized():
            return
        if self.path.startswith(ASSISTANTS_PATH):
            self.reply(OK, PAGE)
            return
        if self.path.startswith(f"{THREADS_PATH}/"):
            self.reply(OK, THREAD)
            return
        self.send_error(NOT_FOUND)

    def do_POST(self) -> None:
        self.record(self.body())
        if not self.authorized():
            return
        route = self.POST_ROUTES.get(self.path)
        if route is None:
            self.send_error(NOT_FOUND)
            return
        self.reply(OK, route)

    def do_DELETE(self) -> None:
        self.record({})
        if not self.authorized():
            return
        if self.path.startswith(f"{ASSISTANTS_PATH}/"):
            self.reply(OK, DELETED)
            return
        self.send_error(NOT_FOUND)

    def authorized(self) -> bool:
        key = self.headers.get(AUTHORIZATION)
        if key == f"{BEARER}{GOOD_KEY}":
            return True
        if key == f"{BEARER}{RATE_LIMITED_KEY}":
            self.reply(RATE_LIMITED, RATE_LIMITED_BODY)
            return False
        if key == f"{BEARER}{FAILING_KEY}":
            self.reply(SERVER_ERROR, SERVER_ERROR_BODY)
            return False
        payload: dict[str, JsonValue] = {
            "error": {
                "message": f"Incorrect API key provided: {BAD_KEY}.",
                "type": "invalid_request_error",
                "param": None,
                "code": "invalid_api_key",
            },
        }
        self.reply(UNAUTHORIZED, payload)
        return False

    def body(self) -> JsonValue:
        declared = self.headers.get("Content-Length")
        if declared is None:
            return None
        raw = self.rfile.read(int(declared))
        return JSON.validate_json(raw)

    def record(self, body: JsonValue) -> None:
        call: RecordedCall = {
            "method": self.command,
            "path": self.path,
            "headers": dict(self.headers.items()),
            "body": body,
        }
        self.owner.record(call)

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)
