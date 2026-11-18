import json
from datetime import UTC, datetime
from typing import Literal, Protocol, TypedDict

import fastuuid
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.realtime.transformation import BedrockRealtimeConfig
from litellm.types.llms.openai import OpenAIRealtimeStreamSessionEvents as SessionEvents
from litellm.types.realtime import RealtimeResponseTransformInput, RealtimeResponseTypedDict
from pydantic import JsonValue, TypeAdapter, ValidationError

MODEL = "amazon.nova-sonic-v1:0"

JsonObject = dict[str, JsonValue]
JsonAdapter: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
StateAdapter: TypeAdapter[RealtimeResponseTransformInput] = TypeAdapter(
    RealtimeResponseTransformInput
)


class RealtimeConfig(Protocol):
    def transform_realtime_response(
        self,
        message: str,
        model: str,
        logging_obj: Logging,
        state: RealtimeResponseTransformInput,
    ) -> RealtimeResponseTypedDict: ...

    def session_updated_event(
        self, model: str, logging_obj: Logging, modalities: list[str] | None = None
    ) -> SessionEvents: ...


CONFIG_TYPE: type[RealtimeConfig] = BedrockRealtimeConfig


class DispatchStep(TypedDict):
    method: Literal["dispatch"]
    message: str
    state: JsonObject


class SessionUpdatedStep(TypedDict):
    method: Literal["session_updated"]
    trace_id: str
    modalities: list[str] | None


class Input(TypedDict):
    ids: list[str]
    steps: list[DispatchStep | SessionUpdatedStep]


class FixedFast:
    def __init__(self, values: list[str]) -> None:
        self.values = [fastuuid.UUID(value) for value in values]

    def __call__(self) -> fastuuid.UUID:
        return self.values.pop(0)


def make_logging(trace_id: str) -> Logging:
    messages: list[dict[str, str]] = []
    return Logging(
        model="realtime",
        messages=messages,
        stream=True,
        call_type="realtime",
        start_time=datetime(2020, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="function-1",
        litellm_trace_id=trace_id,
    )


def dispatch_step(config: RealtimeConfig, step: DispatchStep, logging_obj: Logging) -> JsonObject:
    state = StateAdapter.validate_python(step["state"])
    try:
        response = config.transform_realtime_response(step["message"], MODEL, logging_obj, state)
    except ValidationError:
        return {"response": None, "raised": "invalid_payload"}
    except AttributeError:
        return {"response": None, "raised": "not_an_object"}
    except TypeError:
        return {"response": None, "raised": "event_not_an_object"}
    return {"response": JsonAdapter.validate_json(json.dumps(response)), "raised": None}


def session_updated_step(config: RealtimeConfig, step: SessionUpdatedStep) -> JsonObject:
    logging_obj = make_logging(step["trace_id"])
    event = config.session_updated_event(MODEL, logging_obj, step["modalities"])
    return {"event": JsonAdapter.validate_json(json.dumps(event))}


def run(input_value: Input) -> JsonValue:
    fastuuid.uuid4 = FixedFast(input_value["ids"])
    config = CONFIG_TYPE()
    logging_obj = make_logging("dispatch")
    steps: list[JsonValue] = []
    for step in input_value["steps"]:
        if step["method"] == "dispatch":
            steps.append(dispatch_step(config, step, logging_obj))
        else:
            steps.append(session_updated_step(config, step))
    return {"steps": steps}
