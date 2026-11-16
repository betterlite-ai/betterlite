import json
from base64 import b64encode
from collections.abc import Callable
from dataclasses import dataclass
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.llms.gemini.realtime.transformation import GeminiRealtimeConfig
from pydantic import JsonValue, TypeAdapter

MODEL = "gemini-2.5-flash"
UsageObject: TypeAlias = dict[str, JsonValue]
AliasAdder: TypeAlias = Callable[[UsageObject], UsageObject]


class Burst(TypedDict):
    count: int
    id_prefix: str
    name_prefix: str


class CallsStep(TypedDict):
    kind: Literal["calls"]
    tool_call: NotRequired[UsageObject]
    burst: NotRequired[Burst]
    response_id: NotRequired[str | None]
    output_item_id: NotRequired[str | None]


class MessageStep(TypedDict):
    kind: Literal["message"]
    message: str


class AudioStep(TypedDict):
    kind: Literal["audio"]
    audio_bytes: int


class UsageStep(TypedDict):
    kind: Literal["standalone_usage"]
    usage: UsageObject


class DoneStep(TypedDict):
    kind: Literal["response_done"]
    frame: UsageObject
    response_id: NotRequired[str | None]
    conversation_id: NotRequired[str | None]
    output_items: NotRequired[list[UsageObject]]
    session_configuration_request: NotRequired[str | None]


class CloseStep(TypedDict):
    kind: Literal["close"]
    model: str


Step: TypeAlias = CallsStep | MessageStep | AudioStep | UsageStep | DoneStep | CloseStep


class Input(TypedDict):
    op: Literal["steps", "aliases"]
    steps: NotRequired[list[Step]]
    usage: NotRequired[UsageObject]


@dataclass
class Trace:
    litellm_trace_id: str


class LiveConfig(Protocol):
    def transform_tool_call_events(
        self,
        tool_call_message: UsageObject,
        response_id: str | None = None,
        output_item_id: str | None = None,
    ) -> list[UsageObject]: ...

    def transform_realtime_request(
        self, message: str, model: str, session_configuration_request: str | None = None
    ) -> list[str]: ...

    def transform_realtime_response(
        self, message: str, model: str, logging_obj: Trace, transform_input: UsageObject
    ) -> UsageObject: ...

    def transform_response_done_event(
        self,
        message: UsageObject,
        current_response_id: str | None,
        current_conversation_id: str | None,
        output_items: list[UsageObject] | None,
        session_configuration_request: str | None = None,
    ) -> UsageObject: ...

    def unbilled_usage_on_session_close(  # ast-grep-ignore: no-long-names
        self, model: str
    ) -> UsageObject | None: ...


INPUT = TypeAdapter(Input)
DOCUMENT = TypeAdapter(JsonValue)


class CountedUuid:
    def __init__(self) -> None:
        self.drawn = 0

    def __call__(self) -> UUID:
        self.drawn += 1
        return UUID(f"{self.drawn:08x}" * 4)


def burst_message(burst: Burst) -> UsageObject:
    calls: list[JsonValue] = [
        {"id": f"{burst['id_prefix']}{index}", "name": f"{burst['name_prefix']}{index}"}
        for index in range(burst["count"])
    ]
    return {"functionCalls": calls}


def client_messages(config: LiveConfig, text: str) -> JsonValue:
    messages = config.transform_realtime_request(text, MODEL)
    return {"messages": [DOCUMENT.validate_json(message) for message in messages]}


def calls_value(config: LiveConfig, step: CallsStep) -> JsonValue:
    tool_call = step.get("tool_call")
    burst = step.get("burst")
    if tool_call is None:
        if burst is None:
            raise ValueError("a calls step needs a tool_call or a burst")
        tool_call = burst_message(burst)
    events = config.transform_tool_call_events(
        tool_call, step.get("response_id"), step.get("output_item_id")
    )
    return {"events": DOCUMENT.validate_python(events)}


def usage_value(config: LiveConfig, step: UsageStep) -> JsonValue:
    transform_input: UsageObject = {
        "current_output_item_id": None,
        "current_response_id": None,
        "current_conversation_id": None,
        "current_delta_chunks": None,
        "current_item_chunks": None,
        "current_delta_type": None,
        "session_configuration_request": None,
    }
    frame: UsageObject = {"usageMetadata": step["usage"]}
    answer = config.transform_realtime_response(
        json.dumps(frame),
        MODEL,
        Trace(litellm_trace_id="trace"),
        transform_input,
    )
    return {"events": answer["response"]}


def done_value(config: LiveConfig, step: DoneStep) -> JsonValue:
    bodies = step.get("output_items")
    items: list[UsageObject] | None = None
    if bodies is not None:
        items = [{"item": body} for body in bodies]
    event = config.transform_response_done_event(
        step["frame"],
        step.get("response_id"),
        step.get("conversation_id"),
        items,
        step.get("session_configuration_request"),
    )
    return {"event": DOCUMENT.validate_python(event)}


def step_value(config: LiveConfig, step: Step) -> JsonValue:
    if step["kind"] == "calls":
        return calls_value(config, step)
    if step["kind"] == "message":
        return client_messages(config, step["message"])
    if step["kind"] == "audio":
        audio = b64encode(bytes(step["audio_bytes"])).decode()
        append: UsageObject = {"type": "input_audio_buffer.append", "audio": audio}
        return {"sent": len(config.transform_realtime_request(json.dumps(append), MODEL))}
    if step["kind"] == "standalone_usage":
        return usage_value(config, step)
    if step["kind"] == "response_done":
        return done_value(config, step)
    closed = config.unbilled_usage_on_session_close(step["model"])
    return {"usage": DOCUMENT.validate_python(closed)}


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    litellm_uuid.uuid4 = CountedUuid()
    config: LiveConfig
    add_aliases: AliasAdder
    config, add_aliases = GeminiRealtimeConfig(), GeminiRealtimeConfig._add_pipecat_usage_detail_aliases  # type: ignore[no-untyped-call,assignment]  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/realtime/transformation.py#L143-L189
    if parsed["op"] == "aliases":
        usage = parsed.get("usage")
        if usage is None:
            raise ValueError("an aliases call needs a usage")
        return {"usage": add_aliases(usage)}
    steps = parsed.get("steps")
    if steps is None:
        raise ValueError("a steps call needs steps")
    return {"steps": [step_value(config, step) for step in steps]}
