import asyncio
import contextlib
import io
import logging
from collections.abc import Awaitable
from typing import Literal, NotRequired, TypedDict

from litellm.exceptions import BudgetExceededError
from litellm.models.organization import LiteLLM_OrganizationTable
from litellm.models.tag import LiteLLM_TagTable
from litellm.models.team import LiteLLM_TeamTable
from litellm.proxy._types import LiteLLM_ProjectTableCachedObj, UserAPIKeyAuth
from litellm.proxy.auth import auth_checks
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.types.proxy.model_access_group_budget import ModelAccessGroupBudget
from pydantic import JsonValue

import auth_tenant_budgets_script as script


class Call(TypedDict):
    function: Literal["project_hard", "project_soft", "organization", "tags", "groups"]
    database: bool
    spends: list[float]
    project: NotRequired[dict[str, JsonValue] | None]
    token: NotRequired[dict[str, JsonValue] | None]
    team: NotRequired[dict[str, JsonValue] | None]
    organization: NotRequired[dict[str, JsonValue] | None]
    org_raises: NotRequired[bool]
    request_body: NotRequired[dict[str, JsonValue]]
    tag_rows: NotRequired[list[dict[str, JsonValue]]]
    matched_model_access_groups: NotRequired[list[str]]
    group_rows: NotRequired[list[dict[str, JsonValue]]]


class Request(TypedDict):
    calls: list[Call]


def project_of(call: Call) -> LiteLLM_ProjectTableCachedObj | None:
    project = call.get("project")
    return None if project is None else LiteLLM_ProjectTableCachedObj.model_validate(project)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3783


def token_of(call: Call) -> UserAPIKeyAuth | None:
    token = call.get("token")
    return None if token is None else UserAPIKeyAuth.model_validate(token)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356


def team_of(call: Call) -> LiteLLM_TeamTable | None:
    team = call.get("team")
    return None if team is None else LiteLLM_TeamTable.model_validate(team)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/team.py#L86


def organization_of(call: Call) -> LiteLLM_OrganizationTable | None:
    organization = call.get("organization")
    return None if organization is None else LiteLLM_OrganizationTable.model_validate(organization)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/organization.py#L12


def tag_rows_of(call: Call) -> dict[str, LiteLLM_TagTable]:
    rows = call.get("tag_rows")
    if rows is None:
        return {}
    return {row.tag_name: row for row in (LiteLLM_TagTable.model_validate(row) for row in rows)}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/tag.py#L16


def group_rows_of(call: Call) -> dict[str, ModelAccessGroupBudget]:
    rows = call.get("group_rows")
    if rows is None:
        return {}
    validated = [ModelAccessGroupBudget.model_validate(row) for row in rows]
    return {row.access_group_name: row for row in validated}


def denial_record(denied: BudgetExceededError) -> JsonValue:
    return {
        "status": denied.status_code,
        "current_cost": denied.current_cost,
        "max_budget": denied.max_budget,
        "entity_type": denied.entity_type,
        "entity_id": denied.entity_id,
    }


def carried_record(token: UserAPIKeyAuth | None) -> JsonValue:
    if token is None:
        return None
    snapshot = token.org_budget_snapshot
    if snapshot is None:
        return None
    return {
        "organization_alias": token.organization_alias,
        "user_api_key_org_spend": snapshot.spend,
        "user_api_key_org_max_budget": snapshot.max_budget,
    }


async def settled(check: Awaitable[None]) -> BudgetExceededError | None:
    try:
        await check
    except BudgetExceededError as denied:
        return denied
    return None


async def hard_answer(
    reader: script.CounterReads, recorder: script.Recorder, denial: BudgetExceededError | None
) -> JsonValue:
    await asyncio.sleep(0)
    return {
        "reads": list(reader.reads),
        "alerts": list(recorder.alerts),
        "denial": None if denial is None else denial_record(denial),
    }


async def project_hard_call(call: Call, recorder: script.Recorder) -> JsonValue:
    reader = script.CounterReads(call["spends"])
    script.install(reader)
    denial = await settled(
        auth_checks._project_max_budget_check(project_of(call), token_of(call), recorder)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6136-L6191
    )
    return await hard_answer(reader, recorder, denial)


async def project_soft_call(call: Call, recorder: script.Recorder) -> JsonValue:
    handler = script.SoftLog()
    proxy_logger = logging.getLogger("LiteLLM Proxy")
    previous_level = proxy_logger.level
    previous_handlers = proxy_logger.handlers
    previous_propagate = proxy_logger.propagate
    proxy_logger.setLevel(logging.DEBUG)
    # LiteLLM writes DEBUG records to stdout, which carries the helper's answer
    proxy_logger.handlers = [handler]
    proxy_logger.propagate = False
    try:
        await auth_checks._project_soft_budget_check(project_of(call), token_of(call), recorder)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6194-L6235
    finally:
        proxy_logger.handlers = previous_handlers
        proxy_logger.propagate = previous_propagate
        proxy_logger.setLevel(previous_level)
    await asyncio.sleep(0)
    return {"logs": list(handler.messages), "alerts": list(recorder.alerts)}


async def organization_call(call: Call, recorder: script.Recorder) -> JsonValue:
    reader = script.CounterReads(call["spends"])
    script.install(reader)
    auth_checks.get_org_object = script.scripted_org_object  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6337-L6345
    token = token_of(call)
    database = script.SeededDatabase() if call["database"] else None
    denial = await settled(
        auth_checks._organization_max_budget_check(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6302-L6398
            token,
            team_of(call),
            database,
            UserApiKeyCache(),
            recorder,
        )
    )
    await asyncio.sleep(0)
    return {
        "reads": list(reader.reads),
        "alerts": list(recorder.alerts),
        "denial": None if denial is None else denial_record(denial),
        "carried": carried_record(token),
    }


async def tags_call(call: Call, recorder: script.Recorder) -> JsonValue:
    reader = script.CounterReads(call["spends"])
    script.install(reader)
    auth_checks.get_tag_objects_batch = script.scripted_tag_objects  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6436-L6442
    body = call.get("request_body")
    denial = await settled(
        auth_checks._tag_max_budget_check(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6401-L6423
            request_body={} if body is None else body,
            prisma_client=script.SeededDatabase() if call["database"] else None,
            user_api_key_cache=UserApiKeyCache(),
            proxy_logging_obj=recorder,
            valid_token=token_of(call),
        )
    )
    return await hard_answer(reader, recorder, denial)


async def groups_call(call: Call, recorder: script.Recorder) -> JsonValue:
    reader = script.CounterReads(call["spends"])
    script.install(reader)
    auth_checks.get_model_access_group_budgets_batch = script.scripted_group_budgets  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6494-L6498
    groups = call.get("matched_model_access_groups")
    denial = await settled(
        auth_checks._model_access_group_max_budget_check(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6470-L6522
            [] if groups is None else groups,
            script.SeededDatabase() if call["database"] else None,
            UserApiKeyCache(),
        )
    )
    return await hard_answer(reader, recorder, denial)


async def answer_for(call: Call) -> JsonValue:
    recorder = script.Recorder()
    script.Script.organization = organization_of(call)
    script.Script.org_raises = call.get("org_raises") is True
    script.Script.tag_rows = tag_rows_of(call)
    script.Script.group_rows = group_rows_of(call)
    match call["function"]:
        case "project_hard":
            return await project_hard_call(call, recorder)
        case "project_soft":
            return await project_soft_call(call, recorder)
        case "organization":
            return await organization_call(call, recorder)
        case "tags":
            return await tags_call(call, recorder)
        case "groups":
            return await groups_call(call, recorder)


async def answer_calls(calls: list[Call]) -> JsonValue:
    answers: list[JsonValue] = []
    for call in calls:
        try:
            answers.append({"ok": True, "value": await answer_for(call)})
        except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6426-L6467
            answers.append({"ok": False})
    return answers


def run(input_value: Request) -> JsonValue:
    calls = input_value["calls"]
    # LiteLLM resolves sys.stdout at emit time, so records below WARNING would precede the answer
    with contextlib.redirect_stdout(io.StringIO()):
        return asyncio.run(answer_calls(calls))
