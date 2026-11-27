from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.management_endpoints.common_daily_activity import ScopeDenied
from litellm.proxy.management_endpoints.internal_user_endpoints import (
    resolve_user_daily_activity_entity_ids,
)
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

Role: TypeAlias = Literal[
    "proxy_admin",
    "proxy_admin_viewer",
    "org_admin",
    "internal_user",
    "internal_user_viewer",
    "team",
    "customer",
]


class Caller(TypedDict):
    user_id: str | None
    user_role: Role | None


class Call(TypedDict):
    user_id: str | None
    caller: Caller


class Input(TypedDict):
    calls: list[Call]


def caller_key(caller: Caller) -> UserAPIKeyAuth:
    role = caller["user_role"]
    return UserAPIKeyAuth(
        user_id=caller["user_id"],
        user_role=None if role is None else LitellmUserRoles(role),
    )


def answer(call: Call) -> JsonValue:
    try:
        entity_ids = resolve_user_daily_activity_entity_ids(
            user_id=call["user_id"],
            user_api_key_dict=caller_key(call["caller"]),
        )
    except HTTPException as error:
        return {
            "raised": {
                "status_code": error.status_code,
                "detail": JSON.validate_python(error.detail),
            }
        }
    if entity_ids is None:
        return {"entity_ids": None}
    match entity_ids:
        case ScopeDenied(status_code=status_code, reason=reason):
            return {"denied": {"status_code": status_code, "reason": reason}}
        case tuple() as selected:
            return {"entity_ids": [*selected]}


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
