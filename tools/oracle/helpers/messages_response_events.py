import logging
import os
from collections.abc import Mapping
from types import SimpleNamespace
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.exceptions import MidStreamFallbackError
from litellm.llms.anthropic.pass_through.responses_adapters import streaming_iterator as adapter
from litellm.types.llms.openai import ResponseAPIUsage
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

REDACTION_ENV: Final = "LITELLM_DISABLE_REDACT_SECRETS"


class FailureSpec(TypedDict):
    kind: Literal["plain", "mid_stream_fallback"]
    text: str
    message: NotRequired[JsonValue]
    type: NotRequired[JsonValue]
    code: NotRequired[JsonValue]
    status_code: NotRequired[JsonValue]
    original_exception: NotRequired["FailureSpec | None"]


class ErrorSpec(TypedDict):
    shape: Literal["object", "value"]
    value: NotRequired[JsonValue]
    message: NotRequired[JsonValue]
    type: NotRequired[JsonValue]
    code: NotRequired[JsonValue]


class ResponseSpec(TypedDict):
    shape: Literal["object", "value"]
    value: NotRequired[JsonValue]
    output: NotRequired[list[JsonValue]]
    status: NotRequired[JsonValue]
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
    shape: Literal["mapping", "attributes", "attributed"]
    fields: NotRequired[dict[str, JsonValue]]
    attributes: NotRequired[AttributesSpec]


class ProcessEventStep(TypedDict):
    function: Literal["process_event"]
    event: EventSpec


class ProjectFailureStep(TypedDict):
    function: Literal["project_failure"]
    failure: FailureSpec


class ErrorChunkStep(TypedDict):
    function: Literal["error_chunk"]
    status_code: int
    message: str


Step: TypeAlias = ProcessEventStep | ProjectFailureStep | ErrorChunkStep


class Input(TypedDict):
    message_id: str
    model: str
    redaction: Literal["enabled", "disabled"]
    steps: list[Step]


class StateView(TypedDict):
    chunks: list[JsonValue]
    current_block_index: int
    item_id_to_block_index: dict[str, int]
    pending_tool_ids: Mapping[str, JsonValue]
    refusal_text: str
    sent_message_start: bool
    sent_message_stop: bool
    stream_failed: bool
    logs: list[dict[str, str]]


class Output(TypedDict):
    results: list[dict[str, JsonValue]]
    state: StateView


EMPTY_ATTRIBUTES: Final[AttributesSpec] = {}
EMPTY_FIELDS: Final[dict[str, JsonValue]] = {}

CHUNK: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
CHUNKS: Final[TypeAdapter[list[JsonValue]]] = TypeAdapter(list[JsonValue])
RESPONSE_USAGE: Final[TypeAdapter[ResponseAPIUsage]] = TypeAdapter(ResponseAPIUsage)
PROCESS_EVENT: Final = adapter.AnthropicResponsesStreamWrapper._process_event  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L209
FAILURE_STATUS: Final = adapter._failure_status_and_message  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L78
ERROR_CHUNK: Final = adapter._anthropic_error_chunk  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L88


class StreamFailureError(Exception):
    def __init__(self, spec: FailureSpec) -> None:
        super().__init__(spec["text"])
        self.message: JsonValue = spec.get("message")
        self.code: JsonValue = spec.get("code")
        self.type: JsonValue = spec.get("type")
        self.status_code: JsonValue = spec.get("status_code")


class AttributedEvent(dict[str, JsonValue]):
    def __init__(self, fields: dict[str, JsonValue], spec: AttributesSpec) -> None:
        super().__init__(fields)
        response_spec = spec.get("response")
        self.type: JsonValue = spec.get("type")
        self.item: JsonValue = spec.get("item")
        self.item_id: JsonValue = spec.get("item_id")
        self.delta: JsonValue = spec.get("delta")
        self.summary_index: JsonValue = spec.get("summary_index")
        self.response = None if response_spec is None else response_fixture(response_spec)


class LogCapture(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self.records: list[dict[str, str]] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        if record.levelno >= logging.ERROR:
            self.records.append({"level": record.levelname.lower(), "message": record.getMessage()})
        return False


def build_failure(spec: FailureSpec) -> Exception:
    original = spec.get("original_exception")
    if spec["kind"] == "plain":
        return StreamFailureError(spec)
    return MidStreamFallbackError(
        message=spec["text"],
        model="oracle-model",
        llm_provider="openai",
        original_exception=None if original is None else build_failure(original),
    )


def error_fixture(spec: ErrorSpec) -> JsonValue | SimpleNamespace:
    if spec["shape"] == "value":
        return spec.get("value")
    return SimpleNamespace(
        message=spec.get("message"), type=spec.get("type"), code=spec.get("code")
    )


def response_fixture(spec: ResponseSpec) -> JsonValue | SimpleNamespace:
    if spec["shape"] == "value":
        return spec.get("value")
    output = spec.get("output")
    usage = spec.get("usage")
    error = spec.get("error")
    return SimpleNamespace(
        output=[] if output is None else output,
        status=spec.get("status"),
        usage=None if usage is None else RESPONSE_USAGE.validate_python(usage),
        error=None if error is None else error_fixture(error),
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
    fields = spec.get("fields")
    attributes = spec.get("attributes")
    if spec["shape"] == "mapping":
        return EMPTY_FIELDS if fields is None else fields
    if spec["shape"] == "attributes":
        return attribute_event(EMPTY_ATTRIBUTES if attributes is None else attributes)
    return AttributedEvent(
        EMPTY_FIELDS if fields is None else fields,
        EMPTY_ATTRIBUTES if attributes is None else attributes,
    )


def state_view(wrapper: adapter.AnthropicResponsesStreamWrapper, capture: LogCapture) -> StateView:
    return {
        "chunks": CHUNKS.validate_python(list(wrapper._chunk_queue)),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L136
        "current_block_index": wrapper._current_block_index,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L128
        "item_id_to_block_index": wrapper._item_id_to_block_index,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L130
        "pending_tool_ids": wrapper._pending_tool_ids,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L132
        "refusal_text": wrapper._refusal_text,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L137
        "sent_message_start": wrapper._sent_message_start,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L133
        "sent_message_stop": wrapper._sent_message_stop,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L134
        "stream_failed": wrapper._stream_failed,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L135
        "logs": capture.records,
    }


def ensure_redaction(setting: Literal["enabled", "disabled"]) -> None:
    raw = os.environ.get(REDACTION_ENV)
    disabled = raw is not None and raw.lower() == "true"
    if disabled != (setting == "disabled"):
        raise RuntimeError(f"case env and helper input disagree about {REDACTION_ENV}")


def execute(input_value: Input, capture: LogCapture) -> Output:
    wrapper = adapter.AnthropicResponsesStreamWrapper(
        responses_stream=None,
        model=input_value["model"],
    )
    wrapper._message_id = input_value["message_id"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L125
    results: list[dict[str, JsonValue]] = []
    for step in input_value["steps"]:
        if step["function"] == "process_event":
            PROCESS_EVENT(wrapper, build_event(step["event"]))
            results.append({"function": "process_event", "failed": wrapper._stream_failed})  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/responses_adapters/streaming_iterator.py#L135
        elif step["function"] == "project_failure":
            status, message = FAILURE_STATUS(build_failure(step["failure"]))
            results.append({"function": "project_failure", "status": status, "message": message})
        else:
            chunk = ERROR_CHUNK(step["status_code"], step["message"])
            results.append({"function": "error_chunk", "chunk": CHUNK.validate_python(chunk)})
    return {"results": results, "state": state_view(wrapper, capture)}


def run(input_value: Input) -> Output:
    ensure_redaction(input_value["redaction"])
    capture = LogCapture()
    logger = logging.getLogger("LiteLLM")
    # First, so the capture sees the wrapper's message before litellm's filters redact it.
    logger.filters.insert(0, capture)
    try:
        return execute(input_value, capture)
    finally:
        logger.removeFilter(capture)
