import asyncio
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy import proxy_server
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.common_utils.rbac_utils import check_org_admin_can_generate_keys
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


class Settings(TypedDict, total=False):
    disable_key_generate_for_org_admin: bool


class Call(TypedDict):
    role: Role
    settings: Settings
    org_id: NotRequired[str]


class Input(TypedDict):
    calls: list[Call]


def answer(call: Call) -> JsonValue:
    runtime: dict[str, JsonValue] = {}
    disabled = call["settings"].get("disable_key_generate_for_org_admin")
    if disabled is not None:
        runtime["disable_key_generate_for_org_admin"] = disabled
    # LiteLLM leaves the key out of the runtime map when the proxy admin never set it
    proxy_server.general_settings = runtime
    caller = UserAPIKeyAuth(
        user_role=LitellmUserRoles(call["role"]),
        org_id=call.get("org_id"),
    )
    status: int | None = None
    detail: JsonValue = None
    try:
        asyncio.run(check_org_admin_can_generate_keys(caller))
    except HTTPException as error:
        status = error.status_code
        detail = JSON.validate_python(error.detail)
    outcome: dict[str, JsonValue] = {"status": status, "detail": detail}
    return outcome


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
