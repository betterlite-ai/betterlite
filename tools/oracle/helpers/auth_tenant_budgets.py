import asyncio
import contextlib
import io
import logging
import sys
from collections.abc import Awaitable, Sequence
from datetime import datetime
from types import ModuleType
from typing import ClassVar, Final, Literal, NotRequired, TypedDict

from litellm.exceptions import BudgetExceededError
from litellm.models.organization import LiteLLM_OrganizationTable
from litellm.models.tag import LiteLLM_TagTable
from litellm.models.team import LiteLLM_TeamTable
from litellm.proxy._types import CallInfo, LiteLLM_ProjectTableCachedObj, UserAPIKeyAuth
from litellm.proxy.auth import auth_checks
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.types.proxy.model_access_group_budget import ModelAccessGroupBudget
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
SOFT_CHECK: Final = "_project_soft_budget_check"


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


# the collaborators one call hands to the production functions, and the lookups they answer
class Script:
    organization: ClassVar[LiteLLM_OrganizationTable | None] = None
    org_raises: ClassVar[bool] = False
    tag_rows: ClassVar[dict[str, LiteLLM_TagTable]] = {}
    group_rows: ClassVar[dict[str, ModelAccessGroupBudget]] = {}


class Recorder(ProxyLogging):
    def __init__(self) -> None:
        self.alerts: Final[list[JsonValue]] = []

    @override
    async def budget_alerts(self, type: str, user_info: CallInfo) -> None:  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L3046-L3061
        self.alerts.append({"type": type, "info": JSON.validate_json(user_info.model_dump_json())})


class SoftLog(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.messages: Final[list[str]] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.funcName == SOFT_CHECK:
            self.messages.append(record.getMessage())


class CounterReads:
    def __init__(self, spends: list[float]) -> None:
        self.spends: Final = spends
        self.reads: Final[list[JsonValue]] = []

    async def __call__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2797-L2806
        self,
        counter_key: str,
        fallback_spend: float,
        max_budget: float | None = None,
        window_entity_type: str | None = None,
        window_entity_id: str | None = None,
        window_duration: str | None = None,
        window_start: datetime | None = None,
        *,
        fallback_authoritative: bool = False,
    ) -> float:
        del window_entity_type, window_entity_id, window_duration, window_start
        self.reads.append(
            {
                "counter_key": counter_key,
                "fallback_spend": fallback_spend,
                "max_budget": max_budget,
                "fallback_authoritative": fallback_authoritative,
            }
        )
        if len(self.reads) > len(self.spends):
            raise RuntimeError("the scripted spend ran out")
        return self.spends[len(self.reads) - 1]


# the database the checks only test for presence, since the batch lookups are patched below
class SeededDatabase(PrismaClient):
    def __init__(self) -> None:
        pass


class ProxyServer(ModuleType):
    get_current_spend: CounterReads


async def scripted_org_object(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L4107-L4130
    org_id: str,
    prisma_client: PrismaClient | None,
    user_api_key_cache: UserApiKeyCache,
    proxy_logging_obj: ProxyLogging | None = None,
    include_budget_table: bool = False,
) -> LiteLLM_OrganizationTable | None:
    del prisma_client, user_api_key_cache, proxy_logging_obj, include_budget_table
    if Script.org_raises:
        raise RuntimeError(f"the organization lookup failed for {org_id}")
    organization = Script.organization
    if organization is None or organization.organization_id != org_id:
        return None
    return organization


async def scripted_tag_objects(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2247-L2262
    tag_names: Sequence[str],
    prisma_client: PrismaClient | None,
    user_api_key_cache: UserApiKeyCache,
    parent_otel_span: JsonValue | None = None,
    proxy_logging_obj: ProxyLogging | None = None,
) -> dict[str, LiteLLM_TagTable]:
    del prisma_client, user_api_key_cache, parent_otel_span, proxy_logging_obj
    return {name: row for name, row in Script.tag_rows.items() if name in tag_names}


async def scripted_group_budgets(
    access_group_names: Sequence[str],
    prisma_client: PrismaClient | None,
    user_api_key_cache: UserApiKeyCache,
) -> dict[str, ModelAccessGroupBudget]:
    del prisma_client, user_api_key_cache
    return {name: row for name, row in Script.group_rows.items() if name in access_group_names}


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
    return {row.access_group_name: row for row in (ModelAccessGroupBudget.model_validate(row) for row in rows)}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/model_access_group_budget.py#L7


def database_of(call: Call) -> PrismaClient | None:
    return SeededDatabase() if call["database"] else None


def request_body_of(call: Call) -> dict[str, JsonValue]:
    body = call.get("request_body")
    return {} if body is None else body


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


def install(reader: CounterReads) -> None:
    proxy = ProxyServer(PROXY_MODULE)
    proxy.get_current_spend = reader
    sys.modules[PROXY_MODULE] = proxy


async def hard_answer(reader: CounterReads, recorder: Recorder, denial: BudgetExceededError | None) -> JsonValue:
    await asyncio.sleep(0)
    return {
        "reads": list(reader.reads),
        "alerts": list(recorder.alerts),
        "denial": None if denial is None else denial_record(denial),
    }


async def project_hard_call(call: Call, recorder: Recorder) -> JsonValue:
    reader = CounterReads(call["spends"])
    install(reader)
    denial = await settled(auth_checks._project_max_budget_check(project_of(call), token_of(call), recorder))  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6136-L6191
    return await hard_answer(reader, recorder, denial)


async def project_soft_call(call: Call, recorder: Recorder) -> JsonValue:
    handler = SoftLog()
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


async def organization_call(call: Call, recorder: Recorder) -> JsonValue:
    reader = CounterReads(call["spends"])
    install(reader)
    auth_checks.get_org_object = scripted_org_object  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6337-L6345
    token = token_of(call)
    denial = await settled(
        auth_checks._organization_max_budget_check(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6302-L6398
            token,
            team_of(call),
            database_of(call),
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


async def tags_call(call: Call, recorder: Recorder) -> JsonValue:
    reader = CounterReads(call["spends"])
    install(reader)
    auth_checks.get_tag_objects_batch = scripted_tag_objects  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6436-L6442
    denial = await settled(
        auth_checks._tag_max_budget_check(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6401-L6423
            request_body=request_body_of(call),
            prisma_client=database_of(call),
            user_api_key_cache=UserApiKeyCache(),
            proxy_logging_obj=recorder,
            valid_token=token_of(call),
        )
    )
    return await hard_answer(reader, recorder, denial)


async def groups_call(call: Call, recorder: Recorder) -> JsonValue:
    reader = CounterReads(call["spends"])
    install(reader)
    auth_checks.get_model_access_group_budgets_batch = scripted_group_budgets  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6494-L6498
    groups = call.get("matched_model_access_groups")
    denial = await settled(auth_checks._model_access_group_max_budget_check(groups if groups is not None else [], database_of(call), UserApiKeyCache()))  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6470-L6522
    return await hard_answer(reader, recorder, denial)


async def answer_for(call: Call) -> JsonValue:
    recorder = Recorder()
    Script.organization = organization_of(call)
    raises = call.get("org_raises")
    Script.org_raises = raises is not None and raises
    Script.tag_rows = tag_rows_of(call)
    Script.group_rows = group_rows_of(call)
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
