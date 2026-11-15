from datetime import datetime
from typing import Literal, TypeAlias, TypedDict

import litellm
from litellm.caching.caching import Cache
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.spend_tracking.spend_event import (
    SpendEvent,
    SpendEventBuildError,
    SpendEventCallbackArgs,
    SpendEventDecodeError,
    build_spend_event,
    decode_spend_event,
    is_offloadable_success,
    spend_event_callback_args,
)
from litellm.types.interactions import InteractionsAPIResponse
from litellm.types.utils import LiteLLMBatch
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter, ValidationError

Bucket: TypeAlias = dict[str, JsonValue | UserAPIKeyAuth]
Params: TypeAlias = dict[str, JsonValue | Bucket]
Kind: TypeAlias = Literal["absent", "model", "mapping", "batch", "interactions"]
JSON_VALUE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
CALLBACK_ARGS: TypeAdapter[SpendEventCallbackArgs] = TypeAdapter(SpendEventCallbackArgs)
DATETIME: TypeAdapter[datetime] = TypeAdapter(datetime)


class FixtureModel(BaseModel):
    model_config = ConfigDict(extra="allow")


class TypedAuth(TypedDict):
    budget_reservation: dict[str, JsonValue] | None


class TypedAuths(TypedDict):
    metadata: TypedAuth | None
    litellm_metadata: TypedAuth | None


class BodyResponse(TypedDict):
    kind: Literal["model", "mapping"]
    fields: dict[str, JsonValue]


class AbsentResponse(TypedDict):
    kind: Literal["absent"]


class BuildInput(TypedDict):
    function: Literal["build"]
    kwargs: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    metadata: dict[str, JsonValue] | None
    litellm_metadata: dict[str, JsonValue] | None
    typed_auth: TypedAuths
    response: BodyResponse | AbsentResponse
    start_time: str
    end_time: str
    store_bodies: bool
    cache: Literal["off", "on"]


class OffloadableInput(TypedDict):
    function: Literal["offloadable"]
    kind: Kind


class LineInput(TypedDict):
    function: Literal["decode", "restore"]
    line: str


Input: TypeAlias = BuildInput | OffloadableInput | LineInput


def bucket(entries: dict[str, JsonValue] | None, auth: TypedAuth | None) -> Bucket | None:
    if entries is None:
        return None
    if auth is None:
        return {**entries}
    typed = UserAPIKeyAuth(budget_reservation=auth["budget_reservation"])
    return {**entries, "user_api_key_auth": typed}


def build_response(
    spec: BodyResponse | AbsentResponse,
) -> FixtureModel | dict[str, JsonValue] | None:
    if spec["kind"] == "absent":
        return None
    if spec["kind"] == "model":
        return FixtureModel(**spec["fields"])
    return spec["fields"]


def build_event(input_value: BuildInput) -> JsonValue:
    litellm.cache = Cache() if input_value["cache"] == "on" else None
    auth = input_value["typed_auth"]
    params: Params = {
        **input_value["litellm_params"],
        "metadata": bucket(input_value["metadata"], auth["metadata"]),
        "litellm_metadata": bucket(input_value["litellm_metadata"], auth["litellm_metadata"]),
    }
    kwargs: dict[str, JsonValue | Params] = {**input_value["kwargs"], "litellm_params": params}
    response = build_response(input_value["response"])
    result = build_spend_event(
        kwargs,
        response,
        DATETIME.validate_python(input_value["start_time"]),
        DATETIME.validate_python(input_value["end_time"]),
        input_value["store_bodies"],
    )
    match result:
        case SpendEventBuildError():
            return {"build_error": True}
        case bytes():
            return {
                "offloadable": is_offloadable_success(response),
                "newline_terminated": result.endswith(b"\n"),
                "event": JSON_VALUE.validate_json(result),
            }


def offloadable(kind: Kind) -> JsonValue:
    if kind == "absent":
        return {"offloadable": is_offloadable_success(None)}
    if kind == "model":
        return {"offloadable": is_offloadable_success(FixtureModel())}
    if kind == "mapping":
        return {"offloadable": is_offloadable_success({})}
    if kind == "batch":
        return {"offloadable": is_offloadable_success(LiteLLMBatch.model_construct())}
    return {"offloadable": is_offloadable_success(InteractionsAPIResponse.model_construct())}


def decode_line(function: Literal["decode", "restore"], line: str) -> JsonValue:
    event = decode_spend_event(line.encode())
    match event:
        case SpendEventDecodeError():
            return {"decode_error": True}
        case SpendEvent():
            if function == "decode":
                return {"event": JSON_VALUE.validate_json(event.model_dump_json())}
            try:
                args = spend_event_callback_args(event)
            except ValidationError:
                return {"restore_error": True}
            return {"args": JSON_VALUE.validate_json(CALLBACK_ARGS.dump_json(args))}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "build":
        return build_event(input_value)
    if input_value["function"] == "offloadable":
        return offloadable(input_value["kind"])
    return decode_line(input_value["function"], input_value["line"])
