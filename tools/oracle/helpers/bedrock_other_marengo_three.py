import json
from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.llms.bedrock.embed.twelvelabs_marengo_3_transformation import (
    build_marengo_3_request,
    is_marengo_3_model,
)
from pydantic import BaseModel, ConfigDict, JsonValue

JsonMap: TypeAlias = dict[str, JsonValue]


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")
    model: str | None = None
    input: str = ""
    inference_params: JsonMap = {}
    drop_params: bool = False


def to_json(value: object) -> JsonValue:
    return cast("JsonValue", json.loads(json.dumps(value)))


def run_call(argument: JsonValue) -> JsonValue:
    call = Call.model_validate(argument)
    request = build_marengo_3_request(call.input, call.inference_params, call.drop_params)
    return to_json({"detected": is_marengo_3_model(call.model), "request": request})


HANDLERS: dict[str, Callable[[JsonValue], JsonValue]] = {
    "marengo3_model_detection": run_call,
    "named_media_sources": run_call,
    "required_bucket_owner": run_call,
    "text_versus_timed_options": run_call,
    "reject_27_only_options": run_call,
    "drop_policy": run_call,
    "require_input_media": run_call,
}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    name, argument = next(iter(input_value.items()))
    return HANDLERS[name](argument)
