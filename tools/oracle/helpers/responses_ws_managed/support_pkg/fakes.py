import json
import socket
import socketserver
from collections.abc import AsyncIterator
from copy import deepcopy
from http.server import BaseHTTPRequestHandler
from typing import Final, Literal, TypedDict

from litellm.responses.dispatch import aresponses as real_aresponses
from pydantic import JsonValue, TypeAdapter

DISPATCH_KEY: Final = "sk-fixture"
SCRIPT_EXHAUSTED: Final = "the case scripts no further provider call"
SERIALIZE_MESSAGE: Final = "chunk must be a JSON object"
CREATED_EVENT: Final[dict[str, str]] = {"type": "response.created"}
COMPLETED_EVENT: Final[dict[str, str]] = {"type": "response.completed"}
SSE_BODY: Final = b"".join(
    b"data: " + json.dumps(event).encode() + b"\n\n"
    for event in (CREATED_EVENT, COMPLETED_EVENT))
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Setup(TypedDict):
    model: str
    litellm_metadata: dict[str, JsonValue] | None
    custom_llm_provider: str | None
    api_key: str | None
    api_base: str | None
    timeout: float | None
    first_message: str | None
    kwargs: dict[str, JsonValue]


class ChunkRow(TypedDict):
    kind: Literal["none", "json", "object", "broken"]
    value: JsonValue
    event_type: str | None
    dump: str | None


class ScriptRow(TypedDict):
    fail: str | None
    chunks: list[ChunkRow]
    raise_after: int | None
    raise_message: str


class QuotaRow(TypedDict):
    kind: Literal["rate_limit", "internal"]
    message: str


class SinkRow(TypedDict):
    fails_at: int | None
    message: str


class Input(TypedDict):
    setup: Setup
    frames: list[str]
    disconnect: str
    scripts: list[ScriptRow]
    quota: QuotaRow | None
    sink: SinkRow


class ScriptedError(Exception):
    pass


class Recorder:
    def __init__(self, input_value: Input) -> None:
        self.frames: list[str] = []
        self.sends = 0
        self.calls: list[JsonValue] = []
        self.quotas: list[JsonValue] = []
        self.trace: list[JsonValue] = []
        self.requests: list[JsonValue] = []
        self.incoming = list(input_value["frames"])
        self.incoming_index = 0
        self.disconnect = input_value["disconnect"]
        self.fails_at = input_value["sink"]["fails_at"]
        self.fail_message = input_value["sink"]["message"]


class Answering(BaseHTTPRequestHandler):
    recorder: Recorder

    def __init__(
        self,
        recorder: Recorder,
        request: socket.socket,
        client_address: tuple[str, int],
        server: socketserver.BaseServer,
    ) -> None:
        self.recorder = recorder
        super().__init__(request, client_address, server)

    def do_POST(self) -> None:
        length = self.headers.get("content-length")
        if length is None:
            self.send_error(411)
            return
        body = JSON.validate_json(self.rfile.read(int(length)))
        row: JsonValue = {"method": self.command, "path": self.path, "body": body}
        self.recorder.requests.append(row)
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(SSE_BODY)))
        self.end_headers()
        _written = self.wfile.write(SSE_BODY)


class FakeChunk:
    def __init__(self, *, event_type: str | None, text: str) -> None:
        self.type = event_type
        self.text = text

    def model_dump_json(self, *, exclude_none: bool = True) -> str:
        del exclude_none
        return self.text


class BrokenChunk:
    def model_dump_json(self, *, exclude_none: bool = True) -> str:
        del exclude_none
        raise RuntimeError(SERIALIZE_MESSAGE)


def chunk_of(row: ChunkRow) -> JsonValue | FakeChunk | BrokenChunk:
    kind = row["kind"]
    if kind == "none":
        return None
    if kind == "json":
        return row["value"]
    dump = row["dump"]
    if kind == "broken" or dump is None:
        return BrokenChunk()
    return FakeChunk(event_type=row["event_type"], text=dump)


class FakeStream:
    def __init__(self, recorder: Recorder, script: ScriptRow) -> None:
        self.recorder = recorder
        self.script = script
        self.index = 0

    async def __aiter__(self) -> AsyncIterator[JsonValue | FakeChunk | BrokenChunk]:
        while True:
            if self.script["raise_after"] == self.index:
                raise ScriptedError(self.script["raise_message"])
            if self.index >= len(self.script["chunks"]):
                self.recorder.trace.append("provider-end")
                return
            row = self.script["chunks"][self.index]
            self.index += 1
            yield chunk_of(row)


class ScriptedResponses:
    def __init__(self, recorder: Recorder, scripts: list[ScriptRow], base_url: str) -> None:
        self.recorder = recorder
        self.scripts = scripts
        self.base_url = base_url
        self.index = 0

    async def __call__(self, **kwargs: JsonValue) -> FakeStream:
        call: dict[str, JsonValue] = dict(kwargs)
        model = call.pop("model", None)
        self.recorder.trace.append("provider")
        self.recorder.calls.append({"model": model, "kwargs": call})
        dispatch = deepcopy(call)
        dispatch.update(model=model, api_base=self.base_url, api_key=DISPATCH_KEY, stream=True)
        _answered = await real_aresponses(**dispatch)
        if self.index >= len(self.scripts):
            raise ScriptedError(SCRIPT_EXHAUSTED)
        script = self.scripts[self.index]
        self.index += 1
        failure = script["fail"]
        if failure is not None:
            raise ScriptedError(failure)
        return FakeStream(self.recorder, script)
