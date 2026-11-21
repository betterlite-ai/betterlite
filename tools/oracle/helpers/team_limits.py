from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from litellm.proxy._types import LitellmUserRoles, UpdateTeamRequest, UserAPIKeyAuth
from litellm.proxy.management_endpoints import team_endpoints
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
TEAM_REQUEST: Final[TypeAdapter[UpdateTeamRequest]] = TypeAdapter(UpdateTeamRequest)

RoleName: TypeAlias = Literal[
    "proxy_admin",
    "proxy_admin_viewer",
    "org_admin",
    "internal_user",
    "internal_user_viewer",
    "team",
    "customer",
]


class DefaultTeamParamInput(TypedDict):
    function: Literal["default_team_param"]
    field: str
    params: dict[str, JsonValue] | None


class AvailableTeamInput(TypedDict):
    function: Literal["available_team"]
    team_id: str
    available_teams: list[str] | None
    user_id: str | None
    user_role: RoleName | None


class CreatorAutoAddInput(TypedDict):
    function: Literal["creator_auto_add"]
    user_id: str | None
    user_role: RoleName | None
    disable: bool


class BudgetGuardInput(TypedDict):
    function: Literal["budget_guard"]
    request: JsonValue
    user_role: RoleName | None
    existing: float | None


class ModelCapInput(TypedDict):
    function: Literal["model_cap"]
    request: JsonValue
    user_role: RoleName | None
    existing: dict[str, JsonValue] | None


Input: TypeAlias = (
    DefaultTeamParamInput
    | AvailableTeamInput
    | CreatorAutoAddInput
    | BudgetGuardInput
    | ModelCapInput
)


class DefaultTeamParamAnswer(TypedDict):
    value: JsonValue


class AvailableTeamAnswer(TypedDict):
    available: bool


class CreatorAutoAddAnswer(TypedDict):
    auto_add: bool


class BudgetGuardAnswer(TypedDict):
    status: int | None
    guarded: bool
    expected: JsonValue


class ModelCapAnswer(TypedDict):
    status: int | None


Answer: TypeAlias = (
    DefaultTeamParamAnswer
    | AvailableTeamAnswer
    | CreatorAutoAddAnswer
    | BudgetGuardAnswer
    | ModelCapAnswer
)


class Rejection(BaseModel):
    status_code: int


REJECTION: Final[TypeAdapter[Rejection]] = TypeAdapter(Rejection)


def caller_for(user_id: str | None, user_role: RoleName | None) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        user_id=user_id,
        user_role=None if user_role is None else LitellmUserRoles(user_role),
    )


def default_team_param_answer(input_value: DefaultTeamParamInput) -> DefaultTeamParamAnswer:
    litellm.default_team_params = input_value["params"]
    try:
        value = team_endpoints._get_default_team_param(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L783-L802
            input_value["field"]
        )
    finally:
        litellm.default_team_params = None
    return {"value": _JSON.validate_python(value)}


def available_team_answer(input_value: AvailableTeamInput) -> AvailableTeamAnswer:
    teams = input_value["available_teams"]
    litellm.default_internal_user_params = (
        None if teams is None else {"available_teams": teams}
    )
    caller = caller_for(input_value["user_id"], input_value["user_role"])
    try:
        available = team_endpoints._is_available_team(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L805-L810
            input_value["team_id"], caller
        )
    finally:
        litellm.default_internal_user_params = None
    return {"available": available}


def creator_auto_add_answer(input_value: CreatorAutoAddInput) -> CreatorAutoAddAnswer:
    caller = caller_for(input_value["user_id"], input_value["user_role"])
    general_settings: dict[str, JsonValue] = {
        "disable_auto_add_proxy_admin_to_teams": input_value["disable"]
    }
    auto_add = team_endpoints._should_auto_add_team_creator(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L1318-L1326
        caller, general_settings
    )
    return {"auto_add": auto_add}


def budget_guard_answer(input_value: BudgetGuardInput) -> BudgetGuardAnswer:
    request = TEAM_REQUEST.validate_python(input_value["request"])
    caller = caller_for(None, input_value["user_role"])
    try:
        guard = team_endpoints._check_team_budget_update_authority(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L1188-L1229
            request, caller, input_value["existing"]
        )
    except HTTPException as error:
        rejected = REJECTION.validate_python(error, from_attributes=True)
        return {"status": rejected.status_code, "guarded": False, "expected": None}
    if guard is None:
        return {"status": None, "guarded": False, "expected": None}
    return {
        "status": None,
        "guarded": True,
        "expected": _JSON.validate_python(guard.expected),
    }


def model_cap_answer(input_value: ModelCapInput) -> ModelCapAnswer:
    request = TEAM_REQUEST.validate_python(input_value["request"])
    caller = caller_for(None, input_value["user_role"])
    try:
        team_endpoints._check_team_model_budget_update_authority(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L1269-L1315
            request, caller, input_value["existing"]
        )
    except HTTPException as error:
        rejected = REJECTION.validate_python(error, from_attributes=True)
        return {"status": rejected.status_code}
    return {"status": None}


def run(input_value: Input) -> Answer:
    match input_value["function"]:
        case "default_team_param":
            return default_team_param_answer(input_value)
        case "available_team":
            return available_team_answer(input_value)
        case "creator_auto_add":
            return creator_auto_add_answer(input_value)
        case "budget_guard":
            return budget_guard_answer(input_value)
        case "model_cap":
            return model_cap_answer(input_value)
