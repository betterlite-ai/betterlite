import asyncio
import logging
import sys
from types import ModuleType
from typing import Final, Literal, NotRequired, TypedDict
from unittest.mock import patch

import httpx
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.constants import DEFAULT_IN_MEMORY_TTL
from litellm.models.budget import LiteLLM_BudgetTable
from litellm.models.organization import LiteLLM_OrganizationTable
from litellm.proxy._types import LiteLLM_ProjectTableCachedObj
from litellm.proxy.auth.auth_checks import (
    OrganizationNotFoundError,
    get_org_object,
    get_org_object_by_alias,
    get_org_object_for_request,
    get_project_object,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
MANAGEMENT_TTL_SECONDS: Final = 60.0
OUTAGE: Final = "outage"
FAULT: Final = "fault"
LOGGING_FUNCTIONS: Final = frozenset(("get_org_object_by_alias", "get_org_object_for_request"))
FLOAT: Final[TypeAdapter[float]] = TypeAdapter(float)

Function = Literal["org_object", "org_alias", "org_for_request", "project_object"]
Detail = Literal["object", "text"]
Row = dict[str, JsonValue]


class Step(TypedDict):
    now: NotRequired[float]
    fail: NotRequired[list[str]]
    allow: NotRequired[bool | str | None]


class Call(TypedDict):
    function: Function
    database: bool
    target: str
    include_budget: NotRequired[bool]
    in_memory_ttl: float
    cached_org: NotRequired[Row]
    cached_last_known: NotRequired[Row]
    cached_project: NotRequired[Row]
    org_row: NotRequired[Row]
    alias_rows: NotRequired[list[Row]]
    project_row: NotRequired[Row]
    steps: list[Step]


class BudgetView(TypedDict):
    max_budget: float | None
    allowed_models: list[str] | None


class OrgView(TypedDict):
    organization_id: str | None
    organization_alias: str | None
    models: list[str]
    budget: BudgetView | None


class ProjectView(TypedDict):
    project_id: str
    project_alias: str | None
    budget: BudgetView | None
    refreshed_at: float | None


class Failure(TypedDict):
    kind: Literal["org_not_found", "http", "exception"]
    status: int | None
    detail: Detail | None


class StepAnswer(TypedDict):
    value: OrgView | ProjectView | None
    error: Failure | None
    events: list[str]
    logs: list[str]


class Answer(TypedDict):
    steps: list[StepAnswer]


Result = OrgView | ProjectView | None


def org_key(org_id: str, include_budget: bool) -> str:
    if include_budget:
        return f"org_id:{org_id}:with_budget"
    return f"org_id:{org_id}"


def last_known_key(org_id: str) -> str:
    return f"org_id:{org_id}:with_budget:last_known"


def project_key(project_id: str) -> str:
    return f"project_id:{project_id}"


def budget_view(budget: LiteLLM_BudgetTable | None) -> BudgetView | None:
    if budget is None:
        return None
    return BudgetView(max_budget=budget.max_budget, allowed_models=budget.allowed_models)


def org_view(org: LiteLLM_OrganizationTable | None) -> OrgView | None:
    if org is None:
        return None
    return OrgView(
        organization_id=org.organization_id,
        organization_alias=org.organization_alias,
        models=org.models,
        budget=budget_view(org.litellm_budget_table),
    )


def project_view(project: LiteLLM_ProjectTableCachedObj | None) -> ProjectView | None:
    if project is None:
        return None
    return ProjectView(
        project_id=project.project_id,
        project_alias=project.project_alias,
        budget=budget_view(project.litellm_budget_table),
        refreshed_at=project.last_refreshed_at,
    )


def seeds(call: Call) -> dict[str, JsonValue]:
    values: dict[str, JsonValue] = {}
    target = call["target"]
    match call["function"]:
        case "org_object":
            org = call.get("cached_org")
            if org is not None:
                values[org_key(target, call.get("include_budget", False))] = org
        case "org_alias":
            org = call.get("cached_org")
            if org is not None:
                values[f"org_alias:{target}"] = org
        case "org_for_request":
            org = call.get("cached_org")
            if org is not None:
                values[org_key(target, True)] = org
            stale = call.get("cached_last_known")
            if stale is not None:
                values[last_known_key(target)] = stale
        case "project_object":
            project = call.get("cached_project")
            if project is not None:
                values[project_key(target)] = project
    return values


def settings(allow: bool | str | None) -> dict[str, object]:
    values: dict[str, object] = {}
    values["allow_requests_on_db_unavailable"] = allow
    return values


class Fixture(InMemoryCache):
    def __init__(self, call: Call) -> None:
        super().__init__()
        self.events: list[str] = []
        self.logs: list[str] = []
        self.fail: list[str] = []
        self.values: dict[str, JsonValue] = seeds(call)

    def begin(self, step: Step) -> None:
        self.events = []
        self.logs = []
        self.fail = list(step.get("fail", []))

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        self.events.append(f"read {key}")
        return self.values.get(key)

    @override
    def set_cache(self, key: str, value: JsonValue, **kwargs: JsonValue) -> None:
        ttl = kwargs.get("ttl")
        if ttl is None:
            raise ValueError("a management object write carries a ttl")
        seconds = FLOAT.validate_python(ttl)
        self.events.append(f"write {key} ttl={seconds:g}")
        self.values[key] = value


class KeyCache(UserApiKeyCache):
    def __init__(self, fixture: Fixture) -> None:
        super().__init__(in_memory_cache=fixture, default_in_memory_ttl=MANAGEMENT_TTL_SECONDS)
        self.fixture = fixture

    @override
    async def async_set_cache(
        self, key: str | None, value: object, local_only: bool = False, **kwargs: object
    ) -> None:
        if key is not None and f"write {key}" in self.fixture.fail:
            raise RuntimeError(key)
        await super().async_set_cache(key, value, local_only=local_only, **kwargs)


class StoredRow(BaseModel):
    model_config = ConfigDict(extra="allow")


class Table:
    def __init__(self, fixture: Fixture, name: str, rows: list[Row]) -> None:
        self.fixture = fixture
        self.name = name
        self.rows = rows

    async def find_unique(
        self, *, where: dict[str, str], include: dict[str, bool] | None = None
    ) -> StoredRow | None:
        field, value = next(iter(where.items()))
        found = "true" if include else "false"
        self.fixture.events.append(f"find_unique {self.name} {value} include={found}")
        self.refuse()
        for row in self.rows:
            if row.get(field) == value:
                return StoredRow.model_validate(self.visible(row, include))
        return None

    async def find_many(
        self, *, where: dict[str, str], include: dict[str, bool] | None = None
    ) -> list[StoredRow]:
        field, value = next(iter(where.items()))
        self.fixture.events.append(f"find_many {self.name} {value}")
        self.refuse()
        return [
            StoredRow.model_validate(self.visible(row, include))
            for row in self.rows
            if row.get(field) == value
        ]

    def refuse(self) -> None:
        if OUTAGE in self.fixture.fail:
            raise httpx.ConnectError("the authentication database is unreachable")
        if FAULT in self.fixture.fail:
            raise RuntimeError("the authentication database read failed")

    def visible(self, row: Row, include: dict[str, bool] | None) -> Row:
        kept = dict(row)
        if not include:
            kept.pop("litellm_budget_table", None)
        return kept


class FakeDb(PrismaWrapper):
    def __init__(self, tables: dict[str, Table]) -> None:
        self.tables = tables

    @override
    def __getattr__(self, name: str) -> Table:
        try:
            return self.tables[name]
        except KeyError as error:
            raise AttributeError(name) from error


class FakePrismaClient(PrismaClient):
    def __init__(self, tables: dict[str, Table]) -> None:
        self.db = FakeDb(tables)


class ServiceLogger:
    async def async_service_success_hook(self, **_kwargs: object) -> None:
        return None

    async def async_service_failure_hook(self, **_kwargs: object) -> None:
        return None


class Logging:
    def __init__(self) -> None:
        self.service_logging_obj = ServiceLogger()


class ProxyServer(ModuleType):
    general_settings: dict[str, object]
    proxy_logging_obj: Logging


class Board(logging.Handler):
    def __init__(self, fixture: Fixture) -> None:
        super().__init__(logging.DEBUG)
        self.fixture = fixture

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.funcName in LOGGING_FUNCTIONS:
            self.fixture.logs.append(record.levelname)


def client_for(call: Call, fixture: Fixture) -> FakePrismaClient:
    org_rows: list[Row] = []
    org_row = call.get("org_row")
    if org_row is not None:
        org_rows.append(org_row)
    org_rows.extend(call.get("alias_rows", []))
    project_rows: list[Row] = []
    project_row = call.get("project_row")
    if project_row is not None:
        project_rows.append(project_row)
    tables = {
        "litellm_organizationtable": Table(fixture, "organization", org_rows),
        "litellm_projecttable": Table(fixture, "project", project_rows),
    }
    return FakePrismaClient(tables)


def database(call: Call, client: FakePrismaClient) -> FakePrismaClient | None:
    return client if call["database"] else None


async def looked_up(call: Call, client: FakePrismaClient, cache: KeyCache) -> Result:
    target = call["target"]
    match call["function"]:
        case "org_object":
            org = await get_org_object(
                org_id=target,
                prisma_client=database(call, client),
                user_api_key_cache=cache,
                include_budget_table=call.get("include_budget", False),
            )
            return org_view(org)
        case "org_alias":
            return org_view(await get_org_object_by_alias(target, database(call, client), cache))
        case "org_for_request":
            if not call["database"]:
                raise ValueError("the request lookup runs only with a database")
            return org_view(await get_org_object_for_request(target, client, cache, None, None))
        case "project_object":
            project = await get_project_object(target, database(call, client), cache)
            return project_view(project)


async def performed(
    call: Call, client: FakePrismaClient, cache: KeyCache
) -> tuple[Result, Failure | None]:
    try:
        return await looked_up(call, client, cache), None
    except OrganizationNotFoundError:
        return None, Failure(kind="org_not_found", status=None, detail=None)
    except HTTPException as raised:
        detail: Detail = "object" if isinstance(raised.detail, dict) else "text"
        return None, Failure(kind="http", status=raised.status_code, detail=detail)
    except (httpx.ConnectError, RuntimeError):
        return None, Failure(kind="exception", status=None, detail=None)


async def answer(call: Call, fixture: Fixture) -> Answer:
    cache = KeyCache(fixture)
    client = client_for(call, fixture)
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.proxy_logging_obj = Logging()
    previous = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = proxy_server
    steps: list[StepAnswer] = []
    try:
        for step in call["steps"]:
            fixture.begin(step)
            proxy_server.general_settings = settings(step.get("allow"))
            with patch("time.time", return_value=step.get("now", 0.0)):
                value, error = await performed(call, client, cache)
            steps.append(
                StepAnswer(
                    value=value,
                    error=error,
                    events=list(fixture.events),
                    logs=list(fixture.logs),
                )
            )
    finally:
        if previous is None:
            sys.modules.pop(PROXY_MODULE, None)
        else:
            sys.modules[PROXY_MODULE] = previous
    return Answer(steps=steps)


def run(input_value: Call) -> Answer:
    if input_value["in_memory_ttl"] != DEFAULT_IN_MEMORY_TTL:
        raise ValueError("in_memory_ttl must equal the DEFAULT_IN_MEMORY_TTL the case env sets")
    fixture = Fixture(input_value)
    logger = logging.getLogger("LiteLLM Proxy")
    level = logger.level
    handlers = list(logger.handlers)
    logger.setLevel(logging.DEBUG)
    logger.handlers = [Board(fixture)]
    try:
        return asyncio.run(answer(input_value, fixture))
    finally:
        logger.handlers = handlers
        logger.setLevel(level)
