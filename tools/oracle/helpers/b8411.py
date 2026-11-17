from collections.abc import Callable
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy._types import (
    CoordinationRedisParams,
    CostEstimateRequest,
    KeyRequest,
    NewCustomerRequest,
    NewUserRequest,
    ResetSpendRequest,
    UpdateUserRequest,
)
from pydantic import BaseModel, JsonValue, TypeAdapter

_JSON: Final = TypeAdapter(JsonValue)
_OBJECT: Final = TypeAdapter(dict[str, JsonValue])
_NAMES: Final = TypeAdapter(list[JsonValue])
# The user classes carry Any through GenerateRequestBase, so adapt from the type form.
_NEW_USER: Final[TypeAdapter[NewUserRequest]] = TypeAdapter(NewUserRequest)
_UPDATE_USER: Final[TypeAdapter[UpdateUserRequest]] = TypeAdapter(UpdateUserRequest)

Function: TypeAlias = Literal[
    "key",
    "reset_spend",
    "new_user",
    "update_user",
    "customer",
    "cost_estimate",
    "redis_params",
]

NEW_USER_KEYS: Final[tuple[str, ...]] = (
    "user_email",
    "user_alias",
    "user_role",
    "max_budget",
    "teams",
    "auto_create_key",
    "send_invite_email",
    "sso_user_id",
    "organizations",
)

UPDATE_KEYS: Final[tuple[str, ...]] = (
    "password",
    "spend",
    "metadata",
    "user_alias",
    "user_role",
    "max_budget",
)

CUSTOMER_KEYS: Final[tuple[str, ...]] = (
    "user_id",
    "alias",
    "blocked",
    "budget_id",
    "spend",
    "allowed_model_region",
    "default_model",
    "max_budget",
    "soft_budget",
    "max_parallel_requests",
    "tpm_limit",
    "rpm_limit",
    "tpd_limit",
    "budget_duration",
)

BUDGET_KEYS: Final[tuple[str, ...]] = (
    "budget_id",
    "max_budget",
    "soft_budget",
    "max_parallel_requests",
    "tpm_limit",
    "rpm_limit",
    "tpd_limit",
    "budget_duration",
    "model_max_budget",
    "budget_reset_at",
)


class Call(TypedDict):
    function: Function
    body: JsonValue


class Input(TypedDict):
    calls: list[Call]


def object_dump(model: BaseModel) -> dict[str, JsonValue]:
    text = model.model_dump_json()
    return _OBJECT.validate_json(text)


def supplied_names(model_fields_set: set[str], keys: tuple[str, ...]) -> JsonValue:
    names = [name for name in sorted(model_fields_set) if name in keys]
    return _NAMES.validate_python(names)


def key_answer(call: Call) -> JsonValue:
    request = KeyRequest.model_validate(call["body"])
    return object_dump(request)


def reset_spend_answer(call: Call) -> JsonValue:
    request = ResetSpendRequest.model_validate(call["body"])
    return object_dump(request)


def cost_estimate_answer(call: Call) -> JsonValue:
    request = CostEstimateRequest.model_validate(call["body"])
    return object_dump(request)


def new_user_answer(call: Call) -> JsonValue:
    request = _NEW_USER.validate_python(call["body"])
    dump = object_dump(request)
    return {key: dump[key] for key in NEW_USER_KEYS}


def update_user_answer(call: Call) -> JsonValue:
    request = _UPDATE_USER.validate_python(call["body"])
    dump = object_dump(request)
    view: dict[str, JsonValue] = {
        "user_id": dump["user_id"],
        "user_email": dump["user_email"],
        "supplied": supplied_names(request.model_fields_set, UPDATE_KEYS),
    }
    return view


def customer_answer(call: Call) -> JsonValue:
    request = NewCustomerRequest.model_validate(call["body"])
    dump = object_dump(request)
    view: dict[str, JsonValue] = {key: dump[key] for key in CUSTOMER_KEYS}
    view["supplied"] = supplied_names(request.model_fields_set, BUDGET_KEYS)
    return view


def redis_params_answer(call: Call) -> JsonValue:
    params = CoordinationRedisParams.model_validate(call["body"])
    view: dict[str, JsonValue] = {
        "params": object_dump(params),
        "has_connection_target": params.has_connection_target(),
    }
    return view


ANSWERS: Final[dict[Function, Callable[[Call], JsonValue]]] = {
    "key": key_answer,
    "reset_spend": reset_spend_answer,
    "new_user": new_user_answer,
    "update_user": update_user_answer,
    "customer": customer_answer,
    "cost_estimate": cost_estimate_answer,
    "redis_params": redis_params_answer,
}


def call_outcome(call: Call) -> JsonValue:
    try:
        return {"ok": True, "value": ANSWERS[call["function"]](call)}
    except (ValueError, TypeError, KeyError):
        return {"ok": False}


def run(input_value: Input) -> JsonValue:
    outcomes = [call_outcome(call) for call in input_value["calls"]]
    return _JSON.validate_python(outcomes)
