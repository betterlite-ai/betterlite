import asyncio
from typing import Literal, TypedDict

import litellm
from litellm.models.organization import LiteLLM_OrganizationTable
from litellm.models.team import LiteLLM_TeamTable
from litellm.proxy._types import ModelAccessDeniedProxyException as ModelDenied
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth.auth_checks import (
    can_key_call_model,
    can_org_access_model,
    can_team_access_model,
    key_model_aliases_for_auth_check,
)
from litellm.router import Router
from litellm.types.router import RouterModelGroupAliasItem
from pydantic import JsonValue


class DeploymentInput(TypedDict):
    model_name: str
    access_groups: list[JsonValue]
    team_id: str | None
    team_public_model_name: str | None


class Step(TypedDict):
    object_kind: Literal["key", "team", "org"]
    model: str
    models: list[str]
    team_id: str | None
    team_models: list[str]
    team_model_aliases: dict[str, str]
    key_aliases: dict[str, str] | None
    access_group_ids: list[str]
    deployments: list[DeploymentInput]
    model_group_alias: dict[str, str | RouterModelGroupAliasItem]
    global_aliases: dict[str, str]


class Input(TypedDict):
    steps: list[Step]


class ErrorOut(TypedDict):
    error_class: str
    status: int
    error_type: str
    param: str | None
    code: str
    message: str


class StepResult(TypedDict):
    allowed: bool
    error: ErrorOut | None
    key_aliases: dict[str, str] | None


def run(input_value: Input) -> list[StepResult]:
    saved = dict(litellm.model_alias_map)
    saved_suppress = litellm.suppress_debug_info
    # the proxy sets this at startup; without it get_llm_provider prints to stdout, which is the helper's reply channel
    # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L912
    litellm.suppress_debug_info = True
    try:
        return asyncio.run(run_steps(input_value["steps"]))
    finally:
        litellm.model_alias_map = saved
        litellm.suppress_debug_info = saved_suppress


async def run_steps(steps: list[Step]) -> list[StepResult]:
    results: list[StepResult] = []
    for step in steps:
        litellm.model_alias_map = dict(step["global_aliases"])
        results.append(await run_step(step))
    return results


async def run_step(step: Step) -> StepResult:
    router = build_router(step)
    resolved = key_model_aliases_for_auth_check(build_token(step))
    key_aliases = None if resolved is None else dict(resolved)
    try:
        await check_step(step, router, key_aliases)
    except ModelDenied as denial:
        return {"allowed": False, "error": error_out(denial), "key_aliases": key_aliases}
    return {"allowed": True, "error": None, "key_aliases": key_aliases}


async def check_step(step: Step, router: Router | None, key_aliases: dict[str, str] | None) -> None:
    match step["object_kind"]:
        case "key":
            token = build_token(step)
            await can_key_call_model(
                model=step["model"],
                llm_model_list=None,
                valid_token=token,
                llm_router=router,
            )
        case "team":
            team_id = step["team_id"]
            team = (
                None
                if team_id is None
                else LiteLLM_TeamTable(
                    team_id=team_id,
                    models=step["models"],
                    access_group_ids=step["access_group_ids"],
                )
            )
            await can_team_access_model(
                model=step["model"],
                team_object=team,
                llm_router=router,
                team_model_aliases=step["team_model_aliases"],
                key_model_aliases=key_aliases,
            )
        case "org":
            organization = LiteLLM_OrganizationTable(
                budget_id="oracle-budget",
                created_by="oracle-user",
                updated_by="oracle-user",
                models=step["models"],
            )
            can_org_access_model(
                model=step["model"],
                org_object=organization,
                llm_router=router,
                team_model_aliases=step["team_model_aliases"],
            )


def build_token(step: Step) -> UserAPIKeyAuth:
    aliases: dict[str, str] = dict(step["key_aliases"]) if step["key_aliases"] is not None else {}
    return UserAPIKeyAuth(
        models=step["models"],
        team_id=step["team_id"],
        team_models=step["team_models"],
        team_model_aliases=step["team_model_aliases"],
        aliases=aliases,
        access_group_ids=step["access_group_ids"],
    )


def error_out(denial: ModelDenied) -> ErrorOut:
    return {
        "error_class": type(denial).__name__,
        "status": int(denial.code),
        "error_type": denial.type,
        "param": denial.param,
        "code": denial.code,
        "message": denial.message,
    }


def build_router(step: Step) -> Router | None:
    if not step["deployments"] and not step["model_group_alias"]:
        return None
    model_list: list[dict[str, JsonValue]] = []
    for deployment in step["deployments"]:
        info: dict[str, JsonValue] = {"access_groups": deployment["access_groups"]}
        if deployment["team_id"] is not None:
            info["team_id"] = deployment["team_id"]
        if deployment["team_public_model_name"] is not None:
            info["team_public_model_name"] = deployment["team_public_model_name"]
        # the row's backing model id only has to be one the router can place
        params: dict[str, JsonValue] = {"model": "openai/gpt"}
        row: dict[str, JsonValue] = {
            "model_name": deployment["model_name"],
            "litellm_params": params,
            "model_info": info,
        }
        model_list.append(row)
    return Router(model_list=model_list, model_group_alias=dict(step["model_group_alias"]))
