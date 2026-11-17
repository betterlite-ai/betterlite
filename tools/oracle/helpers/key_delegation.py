from typing import Final, Literal, TypeAlias, TypedDict

from fastapi import HTTPException
from litellm.models.team import BudgetLimitEntry, LiteLLM_TeamTableCachedObj
from litellm.models.verification_token import LiteLLM_VerificationToken
from litellm.proxy._types import UpdateKeyRequest, UserAPIKeyAuth
from litellm.proxy.management_endpoints import key_management_endpoints
from pydantic import BaseModel, JsonValue, TypeAdapter

# the checks read presence off the validated model, so the body arrives as the route decodes it
UPDATE: Final[TypeAdapter[UpdateKeyRequest]] = TypeAdapter(UpdateKeyRequest)
# pydantic reads the enum by value, and LiteLLM declares every role in snake_case
CALLER: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)
_JSON: Final = TypeAdapter(JsonValue)

Role: TypeAlias = Literal[
    "proxy_admin",
    "proxy_admin_viewer",
    "org_admin",
    "internal_user",
    "internal_user_viewer",
    "team",
    "customer",
]
AmountValue: TypeAlias = int | float | Literal["nan", "inf", "-inf"]


class CallerInput(TypedDict):
    role: Role | None
    is_session_token: bool
    is_ui_session_team_key: bool


class OwnershipCall(TypedDict):
    check: Literal["ownership"]
    body: JsonValue
    existing_user_id: str | None
    caller: CallerInput


class RoutesCall(TypedDict):
    check: Literal["routes"]
    allowed_routes: list[str] | None
    was_provided: bool
    allow_safe_presets: bool
    caller: CallerInput


class TransitionCall(TypedDict):
    check: Literal["transition"]
    incoming: list[str] | None
    existing: list[str] | None


class RoutesUpdateCall(TypedDict):
    check: Literal["routes_update"]
    body: JsonValue
    existing_allowed_routes: list[str] | None
    caller: CallerInput


class PermissionsCall(TypedDict):
    check: Literal["permissions"]
    body: JsonValue
    caller: CallerInput


class WindowInput(TypedDict):
    budget_duration: str
    max_budget: AmountValue


class BudgetCall(TypedDict):
    check: Literal["budget"]
    budget_limits: list[WindowInput] | None
    delegation_ceiling: AmountValue | None
    caller: CallerInput
    has_team: bool


Call: TypeAlias = (
    OwnershipCall
    | RoutesCall
    | TransitionCall
    | RoutesUpdateCall
    | PermissionsCall
    | BudgetCall
)


class Input(TypedDict):
    calls: list[Call]


class Rejection(BaseModel):
    status_code: int
    detail: str | dict[str, str]


REJECTION: Final[TypeAdapter[Rejection]] = TypeAdapter(Rejection)


def caller_record(input_value: CallerInput) -> UserAPIKeyAuth:
    fields: dict[str, JsonValue] = {
        "user_role": input_value["role"],
        "is_session_token": input_value["is_session_token"],
    }
    return CALLER.validate_python(fields)


def rejection(error: HTTPException) -> JsonValue:
    rejected = REJECTION.validate_python(error, from_attributes=True)
    detail = rejected.detail
    match detail:
        case str():
            return {"ok": False, "status": rejected.status_code, "detail_error": False}
        case dict():
            return {
                "ok": False,
                "status": rejected.status_code,
                "detail_error": "error" in detail,
            }


def ownership_answer(call: OwnershipCall) -> JsonValue:
    data = UPDATE.validate_python(call["body"])
    row = LiteLLM_VerificationToken(user_id=call["existing_user_id"])
    try:
        key_management_endpoints._validate_caller_can_change_key_ownership(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L873-L916
            data=data,
            existing_key_row=row,
            user_api_key_dict=caller_record(call["caller"]),
        )
    except HTTPException as error:
        return rejection(error)
    return {"ok": True}


def routes_answer(call: RoutesCall) -> JsonValue:
    try:
        key_management_endpoints._check_allowed_routes_caller_permission(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L919-L959
            allowed_routes=call["allowed_routes"],
            user_api_key_dict=caller_record(call["caller"]),
            allowed_routes_was_provided=call["was_provided"],
            allow_safe_presets=call["allow_safe_presets"],
        )
    except HTTPException as error:
        return rejection(error)
    return {"ok": True}


def transition_answer(call: TransitionCall) -> JsonValue:
    safe = key_management_endpoints._is_safe_preset_route_transition(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L965-L981
        incoming_allowed_routes=call["incoming"],
        existing_allowed_routes=call["existing"],
    )
    return {"ok": True, "value": safe}


def routes_update_answer(call: RoutesUpdateCall) -> JsonValue:
    data = UPDATE.validate_python(call["body"])
    row = LiteLLM_VerificationToken(allowed_routes=call["existing_allowed_routes"])
    try:
        key_management_endpoints._enforce_allowed_routes_update_permission(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L984-L998
            data=data,
            existing_key_row=row,
            user_api_key_dict=caller_record(call["caller"]),
        )
    except HTTPException as error:
        return rejection(error)
    return {"ok": True}


def permissions_answer(call: PermissionsCall) -> JsonValue:
    data = UPDATE.validate_python(call["body"])
    try:
        key_management_endpoints._check_permissions_caller_permission(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L1001-L1020
            data=data,
            user_api_key_dict=caller_record(call["caller"]),
        )
    except HTTPException as error:
        return rejection(error)
    return {"ok": True}


def budget_answer(call: BudgetCall) -> JsonValue:
    limits = call["budget_limits"]
    windows = (
        None
        if limits is None
        else [
            BudgetLimitEntry(
                budget_duration=window["budget_duration"],
                max_budget=float(window["max_budget"]),
            )
            for window in limits
        ]
    )
    ceiling = call["delegation_ceiling"]
    row = (
        LiteLLM_TeamTableCachedObj(team_id="delegated-team")
        if call["has_team"]
        else None
    )
    try:
        key_management_endpoints._check_budget_limits_delegation_ceiling(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L1023-L1071
            budget_limits=windows,
            delegation_ceiling=None if ceiling is None else float(ceiling),
            user_api_key_dict=caller_record(call["caller"]),
            is_ui_session_team_key=call["caller"]["is_ui_session_team_key"],
            team_table=row,
        )
    except HTTPException as error:
        return rejection(error)
    return {"ok": True}


def answer(call: Call) -> JsonValue:
    if call["check"] == "ownership":
        return ownership_answer(call)
    if call["check"] == "routes":
        return routes_answer(call)
    if call["check"] == "transition":
        return transition_answer(call)
    if call["check"] == "routes_update":
        return routes_update_answer(call)
    if call["check"] == "permissions":
        return permissions_answer(call)
    return budget_answer(call)


def run(input_value: Input) -> JsonValue:
    return _JSON.validate_python([answer(call) for call in input_value["calls"]])
