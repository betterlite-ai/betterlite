import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Self, TypedDict

from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

HOST = "127.0.0.1"
OK = 200
ACCEPTED = 202
UNAUTHORIZED = 401
SLOW_DELAY = 1.0
DENIED_CODE = -32602
INSTRUCTIONS = "  follow the map  "
NORMAL = "normal"
REFUSE = "refuse"
SLOW = "slow"
UNAUTHORIZED_MODE = "unauthorized"


class Message(TypedDict, total=False):
    jsonrpc: str
    id: int
    method: str
    params: dict[str, JsonValue]


class Recording:
    def __init__(self) -> None:
        self.headers: dict[str, str] = {}
        self.methods: list[str] = []
        self.lock = threading.Lock()


class Server(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, mode: str = NORMAL) -> None:
        super().__init__((HOST, 0), _Handler)
        self.mode = mode
        self.record = Recording()
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()
        _Handler.recording = self

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}/mcp"

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
    recording: ClassVar[Server | None] = None

    def do_POST(self) -> None:
        server = self._server()
        declared = self.headers.get("Content-Length")
        length = 0 if declared is None else int(declared)
        raw = self.rfile.read(length)
        message = TypeAdapter(Message).validate_json(raw)
        method = message["method"]
        with server.record.lock:
            server.record.headers = {
                name.lower(): value for name, value in self.headers.items()
            }
            server.record.methods.append(method)
        if server.mode == UNAUTHORIZED_MODE:
            self._reply(UNAUTHORIZED, b"denied")
            return
        if not method.startswith("notifications/"):
            if server.mode == SLOW:
                time.sleep(SLOW_DELAY)
            if server.mode == REFUSE and method in {"tools/call", "tools/list"}:
                self._error(message.get("id"), method)
                return
            self._reply(OK, json.dumps(self._answer(message, method)).encode())
            return
        self.send_response(ACCEPTED)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def _server(self) -> Server:
        server = _Handler.recording
        if server is None:
            raise RuntimeError("the handler is not on a Server")
        return server

    def _reply(self, status: int, payload: bytes) -> None:
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def _error(self, ident: int | None, method: str) -> None:
        payload = {
            "jsonrpc": "2.0",
            "id": ident,
            "error": {"code": DENIED_CODE, "message": f"{method} denied by policy"},
        }
        self._reply(OK, json.dumps(payload).encode())

    def _answer(self, message: Message, method: str) -> dict[str, JsonValue]:
        params: dict[str, JsonValue] = {}
        if "params" in message:
            params = message["params"]
        result = self._result(method, params)
        return {"jsonrpc": "2.0", "id": message.get("id"), "result": result}

    def _result(self, method: str, params: dict[str, JsonValue]) -> dict[str, JsonValue]:
        match method:
            case "initialize":
                return self._initialize(params)
            case "tools/list":
                return self._tools(params)
            case "tools/call":
                return self._call(params)
            case "prompts/list":
                return {"prompts": [{"name": "greet", "description": "say hi"}]}
            case "prompts/get":
                return {
                    "description": "greet",
                    "messages": [
                        {
                            "role": "user",
                            "content": {"type": "text", "text": "hi there"},
                        }
                    ],
                }
            case _:
                return self._resources(method, params)

    def _resources(self, method: str, params: dict[str, JsonValue]) -> dict[str, JsonValue]:
        if method == "resources/read":
            return {"contents": [{"uri": params["uri"], "text": "body"}]}
        if method == "resources/list":
            return {"resources": [{"uri": "file:///notes.txt", "name": "notes"}]}
        if method == "resources/templates/list":
            return {
                "resourceTemplates": [{"uriTemplate": "file:///{name}", "name": "note"}],
            }
        return {}

    def _initialize(self, params: dict[str, JsonValue]) -> dict[str, JsonValue]:
        requested = params["protocolVersion"]
        return {
            "protocolVersion": requested,
            "capabilities": {"prompts": {}, "resources": {}},
            "serverInfo": {"name": "fake-mcp", "version": "1.0"},
            "instructions": INSTRUCTIONS,
        }

    def _tools(self, params: dict[str, JsonValue]) -> dict[str, JsonValue]:
        if params.get("cursor") == "page2":
            return {"tools": [{"name": "fetch", "inputSchema": {"type": "object"}}]}
        return {
            "tools": [
                {
                    "name": "search",
                    "description": "finds things",
                    "inputSchema": {"type": "object", "properties": {}},
                }
            ],
            "nextCursor": "page2",
        }

    def _call(self, params: dict[str, JsonValue]) -> dict[str, JsonValue]:
        name = params["name"]
        return {
            "content": [{"type": "text", "text": f"called {name}"}],
            "isError": False,
        }
