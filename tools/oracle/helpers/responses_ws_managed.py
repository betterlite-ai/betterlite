import json
import logging
import socket
import socketserver
import threading
from datetime import datetime
from functools import partial
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Final, Literal, TypedDict

import litellm
from litellm.exceptions import RateLimitError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import UserAPIKeyAuth
from litellm.responses.dispatch import aresponses as real_aresponses
from litellm.responses.streaming_iterator import (
    ManagedResponsesWebSocketHandler as Handler,
    ProjectQuotaCallback,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

DISPATCH_KEY: Final = "sk-fixture"
SCRIPT_EXHAUSTED: Final = "the case scripts no further provider call"
SERIALIZE_MESSAGE: Final = "chunk must be a JSON object"
CREATED_EVENT: Final[dict[str, str]] = {"type": "response.created"}
COMPLETED_EVENT: Final[dict[str, str]] = {"type": "response.completed"}
SSE_BODY: Final = b"".join(
    b"data: " + json.dumps(event).encode() + b"\n\n"
    for event in (CREATED_EVENT, COMPLETED_EVENT)
)
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


class SerializerError(Exception):
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


# the provider HTTP request LiteLLM makes for a turn, answered with one scripted event
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
        raise SerializerError(SERIALIZE_MESSAGE)


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

    def __aiter__(self) -> "FakeStream":
        return self

    async def __anext__(self) -> JsonValue | FakeChunk | BrokenChunk:
        if self.script["raise_after"] == self.index:
            raise ScriptedError(self.script["raise_message"])
        if self.index < len(self.script["chunks"]):
            row = self.script["chunks"][self.index]
            self.index += 1
            return chunk_of(row)
        self.recorder.trace.append("provider-end")
        raise StopAsyncIteration


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
        dispatch = dict(call)
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

litellm.suppress_debug_info = True

LOG_NAME: Final = "LiteLLM"
DIAGNOSTIC_PREFIX: Final = "ManagedResponsesWS: "


class ClientDisconnectedError(Exception):
    pass


class Socket:
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    async def send_text(self, data: str) -> None:
        self.recorder.sends += 1
        attempt = self.recorder.sends
        if attempt == self.recorder.fails_at:
            raise ScriptedError(self.recorder.fail_message)
        self.recorder.frames.append(data)

    async def receive_text(self) -> str:
        self.recorder.trace.append("receive")
        if self.recorder.incoming_index < len(self.recorder.incoming):
            frame = self.recorder.incoming[self.recorder.incoming_index]
            self.recorder.incoming_index += 1
            return frame
        raise ClientDisconnectedError(self.recorder.disconnect)


class ScriptedQuota(ProjectQuotaCallback):
    def __init__(self, recorder: Recorder, row: QuotaRow) -> None:
        self.recorder = recorder
        self.row = row

    @override
    async def enforce_project_io_token_quota_for_frame(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth | None,
        requested_model: str | None,
        estimated_input_tokens: int,
        estimated_output_tokens: int,
    ) -> None:
        del user_api_key_dict
        self.recorder.trace.append("quota")
        self.recorder.quotas.append(
            {
                "model": requested_model,
                "input_tokens": estimated_input_tokens,
                "output_tokens": estimated_output_tokens,
            }
        )
        kind = self.row["kind"]
        if kind == "rate_limit":
            raise RateLimitError(self.row["message"], llm_provider="", model="")
        if kind == "internal":
            raise ScriptedError(self.row["message"])


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if message.startswith(DIAGNOSTIC_PREFIX):
            self.lines.append(message)


class Probe(Handler):
    def history_view(self) -> dict[str, JsonValue]:
        history: dict[str, JsonValue] = {}
        for response_id, messages in self._session_history.items():
            history[response_id] = JSON.validate_python(messages)
        return history


def build_handler(
    setup: Setup, socket: Socket, recorder: Recorder, quota: QuotaRow | None
) -> Probe:
    callbacks: list[ProjectQuotaCallback] | None = None
    if quota is not None:
        callbacks = [ScriptedQuota(recorder, quota)]
    log_obj = Logging(
        model=setup["model"],
        messages=None,
        stream=True,
        call_type="responses",
        start_time=datetime.fromisoformat("2026-01-01T00:00:00+00:00"),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    return Probe(
        websocket=socket,
        model=setup["model"],
        logging_obj=log_obj,
        litellm_metadata=setup["litellm_metadata"],
        api_key=setup["api_key"],
        api_base=setup["api_base"],
        timeout=setup["timeout"],
        custom_llm_provider=setup["custom_llm_provider"],
        first_message=setup["first_message"],
        user_api_key_dict=None,
        quota_callbacks=callbacks,
        **setup["kwargs"],
    )


def run(input_value: Input) -> dict[str, JsonValue]:
    recorder = Recorder(input_value)
    socket = Socket(recorder)
    server = ThreadingHTTPServer(("127.0.0.1", 0), partial(Answering, recorder))
    base_url = f"http://127.0.0.1:{server.server_address[1]}/v1"
    thread = threading.Thread(
        target=partial(server.serve_forever, poll_interval=0.01), daemon=True
    )
    thread.start()
    provider = ScriptedResponses(recorder, input_value["scripts"], base_url)
    handler = build_handler(input_value["setup"], socket, recorder, input_value["quota"])
    capture = Capture()
    logger = logging.getLogger(LOG_NAME)
    previous_level = logger.level
    previous_handlers = list(logger.handlers)
    for existing in previous_handlers:
        logger.removeHandler(existing)
    logger.addHandler(capture)
    logger.setLevel(logging.DEBUG)
    previous_aresponses = litellm.aresponses
    object.__setattr__(litellm, "aresponses", provider)
    try:
        asyncio.run(handler.run())
    finally:
        object.__setattr__(litellm, "aresponses", previous_aresponses)
        server.shutdown()
        server.server_close()
        logger.removeHandler(capture)
        for existing in previous_handlers:
            logger.addHandler(existing)
        logger.setLevel(previous_level)
    return {
        "frames": [JSON.validate_json(text) for text in recorder.frames],
        "provider_calls": recorder.calls,
        "provider_requests": recorder.requests,
        "quota_calls": recorder.quotas,
        "history": handler.history_view(),
        "trace": recorder.trace,
        "diagnostics": capture.lines,
    }
