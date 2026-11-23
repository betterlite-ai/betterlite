import asyncio
from typing import TypedDict

import litellm
from litellm.proxy._types import LiteLLM_UserTable, ModelAccessDeniedProxyException as ModelDenied
from litellm.proxy.auth.auth_checks import can_user_call_model
from pydantic import JsonValue


class Check(TypedDict):
    model: str | list[str]
    user: dict[str, JsonValue] | None
    key_aliases: dict[str, str]


class Input(TypedDict):
    checks: list[Check]


class DenialView(TypedDict):
    type: str
    param: str | None
    code: str


class CheckView(TypedDict):
    allowed: bool
    error: DenialView | None


def user_row(check: Check) -> LiteLLM_UserTable | None:
    row = check["user"]
    if row is None:
        return None
    return LiteLLM_UserTable.model_validate(row)


async def run_check(check: Check) -> CheckView:
    try:
        await can_user_call_model(
            model=check["model"],
            llm_router=None,
            user_object=user_row(check),
            key_model_aliases=check["key_aliases"],
        )
    except ModelDenied as denial:
        denial_view = DenialView(type=denial.type, param=denial.param, code=denial.code)
        return CheckView(allowed=False, error=denial_view)
    return CheckView(allowed=True, error=None)


async def run_checks(checks: list[Check]) -> list[CheckView]:
    views: list[CheckView] = []
    for check in checks:
        views.append(await run_check(check))
    return views


def run(input_value: Input) -> list[CheckView]:
    saved = litellm.suppress_debug_info
    litellm.suppress_debug_info = True
    try:
        return asyncio.run(run_checks(input_value["checks"]))
    finally:
        litellm.suppress_debug_info = saved
