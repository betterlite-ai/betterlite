import json
import socket
import socketserver
from collections import deque
from collections.abc import AsyncIterator
from copy import deepcopy
from http.server import BaseHTTPRequestHandler
from typing import Final, Literal, TypedDict

from litellm.responses.dispatch import ResponsesResult
from litellm.responses.dispatch import aresponses as real_aresponses
from litellm.responses.streaming_iterator import ResponsesAPIStreamingIterator
from litellm.types.llms.openai import ResponsesAPIStreamingResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

STREAM = TypeAdapter(ResponsesAPIStreamingIterator, config=ConfigDict(arbitrary_types_allowed=True))

DISPATCH_KEY: Final = "sk-fixture"
SERIALIZE_MESSAGE: Final = "chunk must be a JSON object"
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
        self.scripts = deque(input_value["scripts"])
        self.stream = input_value["setup"]["kwargs"].get("stream") is not False
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
        if not self.recorder.scripts:
            self.send_error(500, "the case scripts no further provider call")
            return
        script = self.recorder.scripts.popleft()
        events = script_events(script)
        self.send_response(200)
        if self.recorder.stream:
            self.send_header("Content-Type", "text/event-stream")
            payload = b"".join(b"data: " + json.dumps(event).encode() + b"\n\n" for event in events)
        else:
            self.send_header("Content-Type", "application/json")
            buffered: JsonValue = {
                "id": "buffered",
                "created_at": 0,
                "output": [],
                "model": "gpt-4o-mini",
                "object": "response",
            }
            payload = json.dumps(buffered).encode()
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        _written = self.wfile.write(payload)


class CompletedFixture(TypedDict):
    response: dict[str, JsonValue]


class EventTag(TypedDict):
    type: str


class FailureEvent(TypedDict):
    message: str


class BrokenChunk:
    def model_dump_json(self, *, exclude_none: bool = True) -> str:
        del exclude_none
        raise ScriptedError(SERIALIZE_MESSAGE)


class ObservedStream:
    def __init__(self, source: ResponsesAPIStreamingIterator, recorder: Recorder) -> None:
        self.source = source
        self.recorder = recorder

    async def __aiter__(
        self,
    ) -> AsyncIterator[ResponsesAPIStreamingResponse | BrokenChunk | dict[str, JsonValue] | None]:
        while True:
            try:
                event = await self.source.__anext__()
            except StopAsyncIteration:
                self.recorder.trace.append("provider-end")
                return
            serialized = event.model_dump_json(exclude_none=True)
            kind = TypeAdapter(EventTag).validate_json(serialized)["type"]
            if kind == "fixture.end":
                self.recorder.trace.append("provider-end")
                return
            if kind == "fixture.completed":
                completion = TypeAdapter(CompletedFixture).validate_json(serialized)
                yield {"type": "response.completed", "response": completion["response"]}
            elif kind == "fixture.empty":
                yield None
            elif kind == "fixture.broken":
                yield BrokenChunk()
            elif kind == "fixture.error":
                failure = TypeAdapter(FailureEvent).validate_json(serialized)
                raise ScriptedError(failure["message"])
            else:
                yield event


class ScriptedResponses:
    def __init__(self, recorder: Recorder, base_url: str) -> None:
        self.recorder = recorder
        self.base_url = base_url

    async def __call__(self, **kwargs: JsonValue) -> ResponsesResult | ObservedStream:
        call: dict[str, JsonValue] = dict(kwargs)
        model = call.pop("model", None)
        self.recorder.trace.append("provider")
        self.recorder.calls.append({"model": model, "kwargs": call})
        dispatch = deepcopy(call)
        dispatch.update(model=model, api_base=self.base_url, api_key=DISPATCH_KEY)
        result = await real_aresponses(**dispatch)
        if dispatch.get("stream") is False:
            return result
        return ObservedStream(STREAM.validate_python(result), self.recorder)


def script_events(script: ScriptRow) -> list[JsonValue]:
    if script["fail"] is not None:
        return [{"type": "fixture.error", "message": script["fail"]}]
    events: list[JsonValue] = []
    for index, row in enumerate(script["chunks"]):
        if script["raise_after"] == index:
            events.append({"type": "fixture.error", "message": script["raise_message"]})
            return events
        if row["kind"] == "none":
            events.append({"type": "fixture.empty"})
        elif row["kind"] == "broken":
            events.append({"type": "fixture.broken"})
        elif row["kind"] == "object":
            dump = row["dump"]
            events.append({"type": row["event_type"]} if dump is None else JSON.validate_json(dump))
        else:
            events.append(row["value"])
    events.append(
        {"type": "fixture.error", "message": script["raise_message"]}
        if script["raise_after"] == len(script["chunks"])
        else {"type": "fixture.end"}
    )
    return events
