from typing import NotRequired, Protocol, TypeAlias, TypedDict

from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.llms.gemini.realtime.transformation import GeminiRealtimeConfig
from pydantic import JsonValue, TypeAdapter

DEFAULT_MODEL = "gemini-2.5-flash"
TRACE_ID = "session-log"
UUID_GROUP_STEP = 0x00000001000000010000000100000001

State: TypeAlias = dict[str, JsonValue]


class Input(TypedDict):
    frames: list[str]
    model: NotRequired[str]
    state: NotRequired[State]
    pending_usage: NotRequired[dict[str, JsonValue]]
    unbilled_input_audio_bytes: NotRequired[int]


class TransformResult(TypedDict):
    response: list[JsonValue]
    current_output_item_id: str | None
    current_response_id: str | None
    current_conversation_id: str | None
    current_delta_chunks: list[JsonValue] | None
    current_item_chunks: list[JsonValue] | None
    current_delta_type: str | None
    session_configuration_request: str | None


class Trace:
    def __init__(self, trace_id: str) -> None:
        self.litellm_trace_id = trace_id


class LiveConfig(Protocol):
    _pending_usage_metadata: dict[str, JsonValue] | None
    _unbilled_input_audio_bytes: int

    def transform_realtime_response(
        self,
        message: str,
        model: str,
        logging_obj: Trace,
        transform_input: dict[str, JsonValue],
    ) -> TransformResult: ...


class DrawnUuid:
    def __init__(self) -> None:
        self.drawn = 0

    def __call__(self) -> UUID:
        self.drawn += 1
        return UUID(int=self.drawn * UUID_GROUP_STEP)


INPUT = TypeAdapter(Input)


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    config: LiveConfig = GeminiRealtimeConfig()  # type: ignore[no-untyped-call,assignment]  # pyright: ignore[reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/realtime/transformation.py#L143
    pending = parsed.get("pending_usage")
    if pending is not None:
        config._pending_usage_metadata = pending  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/realtime/transformation.py#L151
    unbilled = parsed.get("unbilled_input_audio_bytes")
    if unbilled is not None:
        config._unbilled_input_audio_bytes = unbilled  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/realtime/transformation.py#L152
    litellm_uuid.uuid4 = DrawnUuid()
    model = parsed.get("model")
    if model is None:
        model = DEFAULT_MODEL
    state = parsed.get("state")
    caller: State = {} if state is None else state
    answers: list[JsonValue] = []
    for text in parsed["frames"]:
        answer = config.transform_realtime_response(
            text,
            model,
            Trace(TRACE_ID),
            {
                "current_output_item_id": caller.get("output_item_id"),
                "current_response_id": caller.get("response_id"),
                "current_conversation_id": caller.get("conversation_id"),
                "current_delta_chunks": caller.get("delta_chunks"),
                "current_item_chunks": caller.get("item_chunks"),
                "current_delta_type": caller.get("delta_type"),
                "session_configuration_request": caller.get("session_configuration_request"),
            },
        )
        caller = {
            "output_item_id": answer["current_output_item_id"],
            "response_id": answer["current_response_id"],
            "conversation_id": answer["current_conversation_id"],
            "delta_chunks": answer["current_delta_chunks"],
            "item_chunks": answer["current_item_chunks"],
            "delta_type": answer["current_delta_type"],
            "session_configuration_request": answer["session_configuration_request"],
        }
        answers.append(
            {
                "events": answer["response"],
                "state": caller,
                "pending_usage": config._pending_usage_metadata,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/realtime/transformation.py#L151
            }
        )
    return {"frames": answers}
