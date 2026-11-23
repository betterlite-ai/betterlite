import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import TracebackType
from typing import ClassVar, Final, Self, TypedDict

from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

HOST = "127.0.0.1"
OK = 200
NOT_FOUND = 404
CARD_PATH = "/.well-known/agent-card.json"
LEGACY_CARD_PATH = "/.well-known/agent.json"
VERSIONED_CARD_PATH = "/agentCard/v1.0"
POST_PATH = "/"
LOCALHOST_URL = "http://localhost:9/"
DEAD_URL = "http://127.0.0.2:9/"
AGENT_NAME = "test-agent"
PROTOCOL_VERSION = "0.3"
REPLY_TEXT = "pong"
ARTIFACT_TEXT = "artifact"
TASK_ID = "task-1"
CONTEXT_ID = "context-1"
MESSAGE_ID = "message-1"
ARTIFACT_ID = "artifact-1"
STATUS_STATE = "TASK_STATE_WORKING"
UNKNOWN_METHOD = -32601
SEND_METHOD = "SendMessage"
STREAM_METHOD = "SendStreamingMessage"
SEND_METHODS = (SEND_METHOD, "message/send")
STREAM_METHODS = (STREAM_METHOD, "message/stream")


class JsonRpcCall(TypedDict, total=False):
    id: str | int
    method: str
    params: JsonValue


class RecordedCall(TypedDict):
    method: str
    path: str
    headers: dict[str, str]
    body: JsonValue


CALL: Final[TypeAdapter[JsonRpcCall]] = TypeAdapter(JsonRpcCall)
BODY: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def error_of(call: JsonRpcCall, method: str | None) -> dict[str, JsonValue]:
    name = "unknown" if method is None else method
    return {
        "jsonrpc": "2.0",
        "id": call.get("id"),
        "error": {"code": UNKNOWN_METHOD, "message": f"unknown method {name}"},
    }


class Recorder:
    def __init__(
        self,
        *,
        card_url: str | None = None,
        legacy_card: bool = False,
        interfaces: bool = False,
        streaming: bool = True,
    ) -> None:
        self.card_url = card_url
        self.legacy_card = legacy_card
        self.interfaces = interfaces
        self.streaming = streaming
        self.calls: list[RecordedCall] = []


class AgentServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, recorder: Recorder) -> None:
        super().__init__((HOST, 0), _Handler)
        self.recorder = recorder
        _Handler.owner = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def url(self) -> str:
        return f"http://{HOST}:{self.server_port}"

    @property
    def base_url(self) -> str:
        return f"{self.url}/"

    def card(self) -> dict[str, JsonValue]:
        url = self.base_url if self.recorder.card_url is None else self.recorder.card_url
        card: dict[str, JsonValue] = {
            "protocolVersion": PROTOCOL_VERSION,
            "name": AGENT_NAME,
            "description": "a scripted A2A agent",
            "url": url,
            "preferredTransport": "JSONRPC",
            "capabilities": {"streaming": self.recorder.streaming},
            "defaultInputModes": ["text"],
            "defaultOutputModes": ["text"],
            "skills": [],
        }
        if self.recorder.interfaces:
            card["supportedInterfaces"] = [
                {"url": url, "protocolBinding": "jsonrpc", "protocolVersion": "1.0"}
            ]
        return card

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
    owner: ClassVar[AgentServer]

    def do_GET(self) -> None:
        owner = self.owner
        owner.recorder.calls.append(
            {
                "method": "GET",
                "path": self.path,
                "headers": {name.lower(): value for name, value in self.headers.items()},
                "body": None,
            }
        )
        card = owner.card()
        serves = (self.path == CARD_PATH and not owner.recorder.legacy_card) or (
            self.path == LEGACY_CARD_PATH and owner.recorder.legacy_card
        )
        if serves:
            self.reply(OK, card)
        else:
            self.reply(NOT_FOUND, {})

    def do_POST(self) -> None:
        owner = self.owner
        declared = self.headers.get("Content-Length")
        raw = b"{}" if declared is None else self.rfile.read(int(declared))
        owner.recorder.calls.append(
            {
                "method": "POST",
                "path": self.path,
                "headers": {name.lower(): value for name, value in self.headers.items()},
                "body": BODY.validate_json(raw),
            }
        )
        if self.path != POST_PATH:
            self.reply(NOT_FOUND, {})
            return
        call = CALL.validate_json(raw)
        method = call.get("method")
        if method in SEND_METHODS:
            message: dict[str, JsonValue] = {
                "message": {
                    "role": "ROLE_AGENT",
                    "parts": [{"text": REPLY_TEXT}],
                    "messageId": MESSAGE_ID,
                }
            }
            answer: dict[str, JsonValue] = {
                "jsonrpc": "2.0",
                "id": call.get("id"),
                "result": message,
            }
            self.reply(OK, answer)
        elif method in STREAM_METHODS:
            self.stream(call)
        else:
            self.reply(OK, error_of(call, method))

    def reply(self, status: int, payload: dict[str, JsonValue]) -> None:
        encoded = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def stream(self, call: JsonRpcCall) -> None:
        self.send_response(OK)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        status: dict[str, JsonValue] = {
            "statusUpdate": {
                "taskId": TASK_ID,
                "contextId": CONTEXT_ID,
                "status": {"state": STATUS_STATE},
            }
        }
        artifact: dict[str, JsonValue] = {
            "artifactUpdate": {
                "taskId": TASK_ID,
                "contextId": CONTEXT_ID,
                "artifact": {"artifactId": ARTIFACT_ID, "parts": [{"text": ARTIFACT_TEXT}]},
                "append": False,
                "lastChunk": True,
            }
        }
        message: dict[str, JsonValue] = {
            "message": {
                "role": "ROLE_AGENT",
                "parts": [{"text": REPLY_TEXT}],
                "messageId": MESSAGE_ID,
            }
        }
        for result in (status, artifact, message):
            frame: dict[str, JsonValue] = {
                "jsonrpc": "2.0",
                "id": call.get("id"),
                "result": result,
            }
            self.wfile.write(f"data: {json.dumps(frame)}\n\n".encode())
            self.wfile.flush()
