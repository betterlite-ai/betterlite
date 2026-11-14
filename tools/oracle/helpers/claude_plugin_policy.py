from dataclasses import dataclass
from typing import Literal, TypeAlias, TypedDict

from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.anthropic_endpoints.claude_code_endpoints.claude_code_marketplace import (
    _require_proxy_admin,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/anthropic_endpoints/claude_code_endpoints/claude_code_marketplace.py#L265-L274
    _validate_plugin_source,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/anthropic_endpoints/claude_code_endpoints/claude_code_marketplace.py#L193-L246
)
from litellm.proxy.anthropic_endpoints.claude_code_endpoints.claude_code_skill_access import (
    skill_visibility,
)
from pydantic import JsonValue
from starlette.exceptions import HTTPException

Role: TypeAlias = Literal["proxy_admin", "proxy_admin_viewer", "internal_user"]


class Caller(TypedDict):
    user_role: Role | None
    key_skills: list[str] | None
    team_skills: list[str] | None


class SkillEntry(TypedDict):
    name: str
    enabled: bool


class ValidateCall(TypedDict):
    function: Literal["validate_source"]
    source: dict[str, str]


class AdminCall(TypedDict):
    function: Literal["require_admin"]
    caller: Caller


class VisibilityCall(TypedDict):
    function: Literal["visibility"]
    caller: Caller | None
    plugins: list[SkillEntry]


Call: TypeAlias = ValidateCall | AdminCall | VisibilityCall


class Input(TypedDict):
    calls: list[Call]


@dataclass
class SkillRecord:
    name: str
    enabled: bool


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}


def answer(call: Call) -> JsonValue:
    match call["function"]:
        case "validate_source":
            return validate_answer(call)
        case "require_admin":
            return admin_answer(call)
        case "visibility":
            return visibility_answer(call)


def validate_answer(call: ValidateCall) -> JsonValue:
    try:
        _validate_plugin_source(call["source"])
    except HTTPException as error:
        return {"ok": False, "status": error.status_code, "value": None}
    return {"ok": True, "status": None, "value": None}


def admin_answer(call: AdminCall) -> JsonValue:
    try:
        _require_proxy_admin(caller_of(call["caller"]))
    except HTTPException as error:
        return {"ok": False, "status": error.status_code, "value": None}
    return {"ok": True, "status": None, "value": None}


def visibility_answer(call: VisibilityCall) -> JsonValue:
    raw = call["caller"]
    visibility = skill_visibility(None if raw is None else caller_of(raw))
    records = [SkillRecord(entry["name"], entry["enabled"]) for entry in call["plugins"]]
    allowed: list[JsonValue] = [
        record.name for record in records if visibility.allows(record)
    ]
    granted: list[JsonValue] = [*sorted(visibility.granted)]
    return {
        "ok": True,
        "status": None,
        "value": {
            "granted": granted,
            "sees_private": visibility.sees_private,
            "allowed": allowed,
        },
    }


def caller_of(raw: Caller) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        user_role=role_of(raw["user_role"]),
        object_permission=permission_of(raw["key_skills"]),
        team_object_permission=permission_of(raw["team_skills"]),
    )


def role_of(raw: Role | None) -> LitellmUserRoles | None:
    match raw:
        case "proxy_admin":
            return LitellmUserRoles.PROXY_ADMIN
        case "proxy_admin_viewer":
            return LitellmUserRoles.PROXY_ADMIN_VIEW_ONLY
        case "internal_user":
            return LitellmUserRoles.INTERNAL_USER
        case None:
            return None


def permission_of(skills: list[str] | None) -> LiteLLM_ObjectPermissionTable | None:
    if skills is None:
        return None
    return LiteLLM_ObjectPermissionTable(object_permission_id="case", skills=skills)
