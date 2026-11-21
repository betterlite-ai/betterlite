import asyncio
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy import proxy_server
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.common_utils.rbac_utils import FeatureName, check_feature_access_for_user
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

Role: TypeAlias = Literal[
    "proxy_admin",
    "proxy_admin_viewer",
    "org_admin",
    "internal_user",
    "internal_user_viewer",
    "team",
    "customer",
]

Switch: TypeAlias = bool | str | None


class GeneralSettings(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    disable_agents_for_internal_users: Switch
    allow_agents_for_team_admins: Switch
    # ast-grep-ignore: no-long-names
    disable_vector_stores_for_internal_users: Switch
    # ast-grep-ignore: no-long-names
    allow_vector_stores_for_team_admins: Switch


class Call(TypedDict):
    role: Role | None
    feature: FeatureName
    general_settings: GeneralSettings


class Input(TypedDict):
    calls: list[Call]


JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
RUNTIME: Final[TypeAdapter[dict[str, Switch]]] = TypeAdapter(dict[str, Switch])


def answer(call: Call) -> JsonValue:
    # the runtime map holds only the keys the case sets, like switches an admin never saved
    proxy_server.general_settings = RUNTIME.validate_python(call["general_settings"])
    proxy_server.prisma_client = None
    role = call["role"]
    caller = UserAPIKeyAuth(user_role=None if role is None else LitellmUserRoles(role))
    status: int | None = None
    detail: JsonValue = None
    try:
        asyncio.run(check_feature_access_for_user(caller, call["feature"]))
    except HTTPException as error:
        status = error.status_code
        detail = JSON.validate_python(error.detail)
    return {"status": status, "detail": detail}


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return JSON.validate_python(answers)
