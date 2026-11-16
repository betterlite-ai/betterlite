import json
import logging
from logging.handlers import MemoryHandler
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.llms.gemini.realtime.transformation import GeminiRealtimeConfig
from litellm.types.llms.openai import OpenAIRealtimeEventTypes, ResponsesAPIStreamEvents
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
DeltaType: TypeAlias = Literal["text", "audio"]
Event: TypeAlias = OpenAIRealtimeEventTypes | ResponsesAPIStreamEvents
ModalityLabel: TypeAlias = Literal[
    "response.text.delta", "response.audio.delta", "response.text.done", "response.audio.done"
]
MODALITY_EVENTS: dict[ModalityLabel, OpenAIRealtimeEventTypes] = {
    "response.text.delta": OpenAIRealtimeEventTypes.RESPONSE_TEXT_DELTA,
    "response.audio.delta": OpenAIRealtimeEventTypes.RESPONSE_AUDIO_DELTA,
    "response.text.done": OpenAIRealtimeEventTypes.RESPONSE_TEXT_DONE,
    "response.audio.done": OpenAIRealtimeEventTypes.RESPONSE_AUDIO_DONE,
}


class MapEventCall(TypedDict):
    op: Literal["map_event"]
    key: str
    value: JsonValue
    delta_type: NotRequired[DeltaType]


class StartCall(TypedDict):
    op: Literal["start"]
    response_id: str
    output_item_id: str
    conversation_id: str
    delta_type: DeltaType
    session_configuration_request: NotRequired[str | None]


class DeltaCall(TypedDict):
    op: Literal["delta"]
    server_content: JsonObject
    output_item_id: str
    response_id: str
    delta_type: DeltaType


class ModalityCall(TypedDict):
    op: Literal["modality"]
    event: ModalityLabel
    delta_type: DeltaType
    frame: NotRequired[JsonObject]
    session_configuration_request: NotRequired[str | None]
    output_item_id: NotRequired[str]
    response_id: NotRequired[str]
    conversation_id: NotRequired[str]
    delta_chunks: NotRequired[list[JsonValue]]


class Step(TypedDict):
    kind: Literal["single", "list"]
    event: NotRequired[JsonValue]
    events: NotRequired[list[JsonValue]]


class UpdateCall(TypedDict):
    op: Literal["update"]
    chunks: Literal["delta", "item"]
    steps: list[Step]


Call: TypeAlias = MapEventCall | StartCall | DeltaCall | ModalityCall | UpdateCall


class Input(TypedDict):
    calls: list[Call]


class ContentConfig(Protocol):
    def map_openai_event(
        self, key: str, value: JsonValue, delta_type: DeltaType | None
    ) -> Event: ...

    def return_new_content_delta_events(  # ast-grep-ignore: no-long-names
        self,
        response_id: str,
        output_item_id: str,
        conversation_id: str,
        *,
        delta_type: DeltaType,
        session_configuration_request: str | None = None,
    ) -> list[JsonValue]: ...

    def transform_content_delta_events(
        self, message: JsonObject, output_item_id: str, response_id: str, delta_type: DeltaType
    ) -> JsonObject: ...

    def handle_openai_modality_event(
        self,
        openai_event: Event,
        json_message: JsonObject,
        state_input: JsonObject,
        delta_type: DeltaType,
    ) -> JsonObject: ...

    def update_current_delta_chunks(
        self, transformed_message: JsonValue, current_delta_chunks: list[JsonValue] | None
    ) -> list[JsonValue] | None: ...

    def update_current_item_chunks(
        self, transformed_message: JsonValue, current_item_chunks: list[JsonValue] | None
    ) -> list[JsonValue] | None: ...


INPUT = TypeAdapter(Input)
DOCUMENT = TypeAdapter(JsonValue)


class CountingUuid:
    def __init__(self) -> None:
        self.counter = 0

    def __call__(self) -> UUID:
        value = f"00000000-0000-4000-8000-{self.counter:012x}"
        self.counter += 1
        return UUID(value)


def map_event(call: MapEventCall, config: ContentConfig) -> JsonValue:
    event = config.map_openai_event(call["key"], call["value"], call.get("delta_type"))
    return {"event": event.value}


def start(call: StartCall, config: ContentConfig) -> JsonValue:
    return {
        "events": config.return_new_content_delta_events(
            response_id=call["response_id"],
            output_item_id=call["output_item_id"],
            conversation_id=call["conversation_id"],
            delta_type=call["delta_type"],
            session_configuration_request=call.get("session_configuration_request"),
        )
    }


def delta(call: DeltaCall, config: ContentConfig) -> JsonValue:
    return config.transform_content_delta_events(
        call["server_content"], call["output_item_id"], call["response_id"], call["delta_type"]
    )


def modality(call: ModalityCall, config: ContentConfig) -> JsonValue:
    frame = call.get("frame")
    frame_object: JsonObject = {} if frame is None else frame
    input_state: JsonObject = {
        "session_configuration_request": call.get("session_configuration_request"),
        "current_output_item_id": call.get("output_item_id"),
        "current_response_id": call.get("response_id"),
        "current_delta_chunks": call.get("delta_chunks"),
        "current_item_chunks": None,
        "current_conversation_id": call.get("conversation_id"),
        "current_delta_type": call["delta_type"],
    }
    return config.handle_openai_modality_event(
        MODALITY_EVENTS[call["event"]], frame_object, input_state, call["delta_type"]
    )


def apply_step(
    config: ContentConfig, step: Step, current: list[JsonValue] | None, *, deltas: bool
) -> list[JsonValue] | None:
    message: JsonValue = step.get("event") if step["kind"] == "single" else step.get("events")
    if deltas:
        return config.update_current_delta_chunks(message, current)
    return config.update_current_item_chunks(message, current)


def update(call: UpdateCall, config: ContentConfig) -> JsonValue:
    current: list[JsonValue] | None = None
    states: list[JsonValue] = []
    for step in call["steps"]:
        current = apply_step(config, step, current, deltas=call["chunks"] == "delta")
        states.append(DOCUMENT.validate_json(json.dumps(current)))
    return {"steps": states}


def call_value(call: Call, config: ContentConfig) -> JsonValue:
    if call["op"] == "map_event":
        return map_event(call, config)
    if call["op"] == "start":
        return start(call, config)
    if call["op"] == "delta":
        return delta(call, config)
    if call["op"] == "modality":
        return modality(call, config)
    return update(call, config)


def answer(call: Call, config: ContentConfig) -> JsonValue:
    litellm_uuid.uuid4 = CountingUuid()
    capture = MemoryHandler(capacity=1000, flushLevel=logging.CRITICAL)
    capture.setLevel(logging.WARNING)
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(capture)
    try:
        value = call_value(call, config)
    finally:
        logger.removeHandler(capture)
    return {"value": value, "warnings": [record.getMessage() for record in capture.buffer]}


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    config: ContentConfig = GeminiRealtimeConfig()  # type: ignore[no-untyped-call,assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/realtime/transformation.py#L143-L160
    return {"calls": [answer(call, config) for call in parsed["calls"]]}
