import json
import logging
from base64 import b64decode
from datetime import datetime, timedelta, timezone
from logging.handlers import MemoryHandler
from typing import Literal, NotRequired, Protocol, TypedDict

from litellm.llms.gemini.common_utils import encode_unserializable_types
from litellm.llms.gemini.realtime.transformation import GeminiRealtimeConfig
from litellm.types.realtime import RealtimeInputAudioTranscriptionUsage as TranscriptionUsage
from pydantic import JsonValue, TypeAdapter

MODEL = "gemini-2.5-flash"
TRANSCRIBE_MODEL = "gemini/gemini-3.5-transcribe-live"
TOOL_NAME_MISSING = "name not found for call_id"


class Stamp(TypedDict):
    year: int
    month: int
    day: int
    hour: int
    minute: int
    second: int
    microsecond: int
    offset_seconds: NotRequired[int | None]


class ToolCall(TypedDict):
    call_id: str
    name: str


class Step(TypedDict):
    kind: Literal["message", "audio_pending", "mime"]
    message: NotRequired[str]
    session_configuration_request: NotRequired[str | None]
    tool_calls: NotRequired[list[ToolCall]]
    format: NotRequired[str]


class SdkCase(TypedDict):
    shape: Literal[
        "bytes",
        "datetime",
        "bytes_list",
        "datetime_list",
        "empty_list",
        "object_bytes",
        "list_object_bytes",
        "list_list_bytes",
        "mixed_list",
    ]
    base64: NotRequired[str]
    datetime: NotRequired[Stamp]
    depth: NotRequired[int]


class Input(TypedDict):
    op: Literal["client", "sdk"]
    steps: NotRequired[list[Step]]
    cases: NotRequired[list[SdkCase]]


class LiveClientConfig(Protocol):
    def transform_realtime_request(
        self, message: str, model: str, session_configuration_request: str | None = None
    ) -> list[str]: ...

    def transform_tool_call_events(self, tool_call_message: dict[str, JsonValue]) -> None: ...

    def unbilled_usage_on_session_close(  # ast-grep-ignore: no-long-names
        self, model: str
    ) -> TranscriptionUsage | None: ...

    def get_audio_mime_type(self, input_audio_format: str = "pcm16") -> str: ...


INPUT = TypeAdapter(Input)
DOCUMENT = TypeAdapter(JsonValue)


def blob(case: SdkCase) -> bytes:
    text = case.get("base64")
    if text is None:
        raise ValueError("this shape needs a base64 value")
    return b64decode(text, validate=True)


def moment_of(case: SdkCase) -> datetime:
    spec = case.get("datetime")
    if spec is None:
        raise ValueError("this shape needs a datetime value")
    at = datetime(
        spec["year"],
        spec["month"],
        spec["day"],
        spec["hour"],
        spec["minute"],
        spec["second"],
        spec["microsecond"],
        tzinfo=timezone.utc,
    )
    offset = spec.get("offset_seconds")
    if offset is None:
        return at.replace(tzinfo=None)
    return at.replace(tzinfo=timezone(timedelta(seconds=offset)))


def sdk_case(case: SdkCase) -> JsonValue:
    depth = case.get("depth")
    level = 0 if depth is None else depth
    shape = case["shape"]
    if shape == "bytes":
        encoded = encode_unserializable_types({"value": blob(case)}, level)
    elif shape == "datetime":
        encoded = encode_unserializable_types({"value": moment_of(case)}, level)
    elif shape == "bytes_list":
        encoded = encode_unserializable_types({"value": [blob(case), blob(case)]}, level)
    elif shape == "datetime_list":
        encoded = encode_unserializable_types({"value": [moment_of(case), moment_of(case)]}, level)
    elif shape == "empty_list":
        encoded = encode_unserializable_types({"value": []}, level)
    elif shape == "object_bytes":
        encoded = encode_unserializable_types({"value": {"inner": blob(case)}}, level)
    elif shape == "list_object_bytes":
        encoded = encode_unserializable_types({"value": [{"inner": blob(case)}]}, level)
    elif shape == "list_list_bytes":
        encoded = encode_unserializable_types({"value": [[blob(case)]]}, level)
    else:
        encoded = encode_unserializable_types({"value": [blob(case), moment_of(case)]}, level)
    try:
        payload = json.dumps(encoded)
    except TypeError:
        return "raw"
    return DOCUMENT.validate_json(payload)


def label(message: str) -> str:
    if TOOL_NAME_MISSING in message:
        return "function_name_missing"
    return message


def message_step(config: LiveClientConfig, step: Step) -> JsonValue:
    seeds = step.get("tool_calls")
    if seeds is not None:
        calls: list[JsonValue] = [{"id": item["call_id"], "name": item["name"]} for item in seeds]
        config.transform_tool_call_events({"functionCalls": calls})
    text = step.get("message")
    if text is None:
        raise ValueError("a message step needs a message")
    messages = config.transform_realtime_request(
        text, MODEL, step.get("session_configuration_request")
    )
    return {"messages": [DOCUMENT.validate_json(message) for message in messages]}


def client_step(config: LiveClientConfig, step: Step) -> JsonValue:
    if step["kind"] == "message":
        return message_step(config, step)
    if step["kind"] == "audio_pending":
        pending = config.unbilled_usage_on_session_close(TRANSCRIBE_MODEL)
        return {"pending": pending is not None}
    format_text = step.get("format")
    if format_text is None:
        raise ValueError("a mime step needs a format")
    return {"mime": config.get_audio_mime_type(format_text)}


def client(input_value: Input) -> JsonValue:
    config: LiveClientConfig = GeminiRealtimeConfig()  # type: ignore[no-untyped-call,assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/realtime/transformation.py#L143-L152
    steps = input_value.get("steps")
    if steps is None:
        raise ValueError("a client call needs steps")
    capture = MemoryHandler(capacity=1000, flushLevel=logging.CRITICAL)
    capture.setLevel(logging.WARNING)
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(capture)
    try:
        values = [client_step(config, step) for step in steps]
    finally:
        logger.removeHandler(capture)
    return {"steps": values, "warnings": [label(record.getMessage()) for record in capture.buffer]}


def sdk(input_value: Input) -> JsonValue:
    cases = input_value.get("cases")
    if cases is None:
        raise ValueError("an sdk call needs cases")
    return {"cases": [sdk_case(case) for case in cases]}


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    if parsed["op"] == "client":
        return client(parsed)
    return sdk(parsed)
