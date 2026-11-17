import json
from dataclasses import dataclass
from typing import NotRequired, Protocol, TypedDict

from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.llms.gemini.realtime.transformation import GeminiRealtimeConfig
from pydantic import JsonValue, TypeAdapter

MODEL = "gemini-2.5-flash"
TRACE_ID = "trace"


class Ids(TypedDict):
    conversation_id: NotRequired[str | None]
    response_id: NotRequired[str | None]
    output_item_id: NotRequired[str | None]


class Input(TypedDict):
    tool_call: dict[str, JsonValue]
    ids: Ids
    frame: NotRequired[dict[str, JsonValue]]
    session_configuration_request: NotRequired[str | None]
    pending_usage: NotRequired[dict[str, JsonValue]]


class Answer(TypedDict):
    response: list[JsonValue]
    current_output_item_id: str | None
    current_response_id: str | None
    current_conversation_id: str | None


@dataclass
class Trace:
    litellm_trace_id: str


class LiveConfig(Protocol):
    _pending_usage_metadata: dict[str, JsonValue] | None

    def transform_realtime_response(
        self,
        message: str,
        model: str,
        logging_obj: Trace,
        transform_input: dict[str, JsonValue],
    ) -> Answer: ...


INPUT = TypeAdapter(Input)


class CountedUuid:
    def __init__(self) -> None:
        self.drawn = 0

    def __call__(self) -> UUID:
        self.drawn += 1
        return UUID(f"{self.drawn:08x}" * 4)


def transform_input(ids: Ids, session_configuration_request: str | None) -> dict[str, JsonValue]:
    return {
        "current_output_item_id": ids.get("output_item_id"),
        "current_response_id": ids.get("response_id"),
        "current_conversation_id": ids.get("conversation_id"),
        "current_delta_chunks": None,
        "current_item_chunks": None,
        "current_delta_type": None,
        "session_configuration_request": session_configuration_request,
    }


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    litellm_uuid.uuid4 = CountedUuid()
    config: LiveConfig = GeminiRealtimeConfig()  # type: ignore[no-untyped-call,assignment]  # noqa: E501  # pyright: ignore[reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/realtime/transformation.py#L143
    ids = parsed["ids"]
    pending = parsed.get("pending_usage")
    if pending is not None:
        seed: dict[str, JsonValue] = {"usageMetadata": pending}
        config.transform_realtime_response(
            json.dumps(seed), MODEL, Trace(litellm_trace_id=TRACE_ID), transform_input(ids, None)
        )
    frame: dict[str, JsonValue] = {"toolCall": parsed["tool_call"]}
    extra = parsed.get("frame")
    if extra is not None:
        frame.update(extra)
    answer = config.transform_realtime_response(
        json.dumps(frame),
        MODEL,
        Trace(litellm_trace_id=TRACE_ID),
        transform_input(ids, parsed.get("session_configuration_request")),
    )
    return {
        "events": answer["response"],
        "ids": {
            "conversation_id": answer["current_conversation_id"],
            "response_id": answer["current_response_id"],
            "output_item_id": answer["current_output_item_id"],
        },
        "pending_usage": config._pending_usage_metadata,  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/realtime/transformation.py#L915
    }
