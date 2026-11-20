import asyncio
import logging
import os
from collections.abc import AsyncIterator
from datetime import UTC, datetime
from types import SimpleNamespace
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.anthropic.pass_through.responses_adapters import streaming_iterator as adapter
from litellm.types.llms.openai import ResponseAPIUsage
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

REDACTION_ENV: Final = "LITELLM_DISABLE_REDACT_SECRETS"
CALL_ID: Final = "call-1"
SERVED_ID: Final = "msg_00000000-0000-0000-0000-000000000007"


class FailureSpec(TypedDict):
    text: str
    message: NotRequired[str]
    status_code: NotRequired[int]


class ErrorSpec(TypedDict):
    message: NotRequired[JsonValue]
    type: NotRequired[JsonValue]
    code: NotRequired[JsonValue]


class ResponseSpec(TypedDict):
    output: NotRequired[list[JsonValue]]
    status: NotRequired[str]
    usage: NotRequired[JsonValue]
    error: NotRequired[ErrorSpec]


class AttributesSpec(TypedDict):
    type: NotRequired[JsonValue]
    item: NotRequired[JsonValue]
    item_id: NotRequired[JsonValue]
    delta: NotRequired[JsonValue]
    summary_index: NotRequired[JsonValue]
    response: NotRequired[ResponseSpec]


class EventSpec(TypedDict):
    shape: Literal["mapping", "attributes"]
    fields: NotRequired[dict[str, JsonValue]]
    attributes: NotRequired[AttributesSpec]


Step: TypeAlias = Literal["next", "drain", "sse"]


class Input(TypedDict):
    model: str
    events: list[EventSpec]
    failure: FailureSpec | None
    logging: bool
    redaction: Literal["enabled", "disabled"]
    steps: list[Step]


class Frame(TypedDict):
    event: str
    data: JsonValue


class Output(TypedDict):
    chunks: list[JsonValue]
    frames: list[Frame | None]
    served_id_logged: bool | None
    logs: list[str]


EMPTY_FIELDS: Final[dict[str, JsonValue]] = {}
EMPTY_ATTRIBUTES: Final[AttributesSpec] = {}
EMPTY_OUTPUT: Final[list[JsonValue]] = []
CHUNK: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
RESPONSE_USAGE: Final[TypeAdapter[ResponseAPIUsage]] = TypeAdapter(ResponseAPIUsage)


class UpstreamFailureError(Exception):
    def __init__(self, spec: FailureSpec) -> None:
        super().__init__(spec["text"])
        self.message: str | None = spec.get("message")
        self.status_code: int | None = spec.get("status_code")


def response_fixture(spec: ResponseSpec) -> SimpleNamespace:
    raw_output = spec.get("output")
    output = EMPTY_OUTPUT if raw_output is None else raw_output
    usage = spec.get("usage")
    error = spec.get("error")
    return SimpleNamespace(
        output=output,
        status=spec.get("status"),
        usage=None if usage is None else RESPONSE_USAGE.validate_python(usage),
        error=None if error is None else error_fixture(error),
    )


def error_fixture(spec: ErrorSpec) -> SimpleNamespace:
    return SimpleNamespace(
        message=spec.get("message"),
        type=spec.get("type"),
        code=spec.get("code"),
    )


def attribute_event(spec: AttributesSpec) -> SimpleNamespace:
    response_spec = spec.get("response")
    return SimpleNamespace(
        type=spec.get("type"),
        item=spec.get("item"),
        item_id=spec.get("item_id"),
        delta=spec.get("delta"),
        summary_index=spec.get("summary_index"),
        response=None if response_spec is None else response_fixture(response_spec),
    )


def build_event(spec: EventSpec) -> dict[str, JsonValue] | SimpleNamespace:
    if spec["shape"] == "mapping":
        return spec.get("fields", EMPTY_FIELDS)
    attributes = spec.get("attributes")
    return attribute_event(EMPTY_ATTRIBUTES if attributes is None else attributes)


def frame_value(raw: bytes) -> Frame:
    text = raw.decode()
    if not text.endswith("\n\n"):
        raise ValueError(f"not an SSE frame: {raw!r}")
    event_line, data_line = text.removesuffix("\n\n").split("\n")
    event = event_line.removeprefix("event: ")
    data = data_line.removeprefix("data: ")
    if event == event_line or data == data_line:
        raise ValueError(f"not an SSE frame: {raw!r}")
    return {"event": event, "data": CHUNK.validate_json(data)}


class ScriptedStream(AsyncIterator[object]):
    def __init__(self, events: list[EventSpec], failure: FailureSpec | None) -> None:
        self._events = [build_event(event) for event in events]
        self._failure = failure
        self._index = 0

    @override
    async def __anext__(self) -> object:
        if self._index < len(self._events):
            event = self._events[self._index]
            self._index += 1
            return event
        failure = self._failure
        if failure is None:
            raise StopAsyncIteration
        self._failure = None
        raise UpstreamFailureError(failure)


class LogCapture(logging.Filter):
    def __init__(self, model: str) -> None:
        super().__init__()
        prefix = f"AnthropicResponsesStreamWrapper: upstream Responses stream for {model}"
        self._wanted = (f"{prefix} failed", f"{prefix} ended without a terminal event")
        self.records: list[str] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        message = record.getMessage()
        if message in self._wanted:
            self.records.append(message)
        return False


def ensure_redaction(setting: Literal["enabled", "disabled"]) -> None:
    raw = os.environ.get(REDACTION_ENV)
    disabled = raw is not None and raw.lower() == "true"
    if disabled != (setting == "disabled"):
        raise RuntimeError(f"case env and helper input disagree about {REDACTION_ENV}")


async def _read_chunk(wrapper: adapter.AnthropicResponsesStreamWrapper, answer: Output) -> bool:
    try:
        chunk = await wrapper.__anext__()
    except StopAsyncIteration:
        answer["chunks"].append(None)
        return False
    answer["chunks"].append(CHUNK.validate_python(chunk))
    return True


async def _read_frame(frames: AsyncIterator[bytes], answer: Output) -> None:
    try:
        frame = await frames.__anext__()
    except StopAsyncIteration:
        answer["frames"].append(None)
        return
    answer["frames"].append(frame_value(frame))


async def _drive(input_value: Input) -> Output:
    logging_obj = Logging(
        model=input_value["model"],
        messages=None,
        stream=True,
        call_type="anthropic_messages",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id=CALL_ID,
        function_id="fn-1",
    )
    wrapper = adapter.AnthropicResponsesStreamWrapper(
        responses_stream=ScriptedStream(input_value["events"], input_value["failure"]),
        model=input_value["model"],
        litellm_logging_obj=logging_obj if input_value["logging"] else None,
    )
    minted = wrapper._message_id  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L125
    wrapper._message_id = SERVED_ID  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L125
    sse_stream: AsyncIterator[bytes] | None = None
    answer: Output = {
        "chunks": [],
        "frames": [],
        "served_id_logged": (
            None
            if not input_value["logging"]
            else logging_obj.streamed_anthropic_message_id == minted
        ),
        "logs": [],
    }
    for step in input_value["steps"]:
        if step == "next":
            await _read_chunk(wrapper, answer)
        elif step == "drain":
            more = True
            while more:
                more = await _read_chunk(wrapper, answer)
        elif step == "sse":
            if sse_stream is None:
                sse_stream = wrapper.async_anthropic_sse_wrapper()
            await _read_frame(sse_stream, answer)
    return answer


def run(input_value: Input) -> Output:
    ensure_redaction(input_value["redaction"])
    capture = LogCapture(input_value["model"])
    logger = logging.getLogger("LiteLLM")
    # First, so the capture reads the wrapper's line before litellm's own filters see the record.
    logger.filters.insert(0, capture)
    answer = asyncio.run(_drive(input_value))
    logger.removeFilter(capture)
    answer["logs"] = capture.records
    return answer
