import asyncio
import logging
import sys
import time
from functools import partial
from types import ModuleType
from typing import Final, Literal, NotRequired, TypedDict

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
LOGGING_FUNCTIONS: Final = frozenset(("get_org_object_by_alias", "get_org_object_for_request"))
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

Function = Literal["org_object", "org_alias", "org_for_request", "project_object"]
Row = dict[str, JsonValue]
Answer = tuple[JsonValue, JsonValue]


class Step(TypedDict):
    now: float
    fail: list[str]
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


def budget_view(budget: LiteLLM_BudgetTable | None) -> JsonValue:
    if budget is None:
        return None
    view = {"max_budget": budget.max_budget, "allowed_models": budget.allowed_models}
    return JSON.validate_python(view)


def org_view(org: LiteLLM_OrganizationTable) -> JsonValue:
    return {
        "organization_id": org.organization_id,
        "organization_alias": org.organization_alias,
        "models": JSON.validate_python(org.models),
        "budget": budget_view(org.litellm_budget_table),
    }


def project_view(project: LiteLLM_ProjectTableCachedObj) -> JsonValue:
    return {
        "project_id": project.project_id,
        "project_alias": project.project_alias,
        "budget": budget_view(project.litellm_budget_table),
        "refreshed_at": project.last_refreshed_at,
    }


class Fixture(InMemoryCache):
    def __init__(self, call: Call) -> None:
        super().__init__()
        self.events: list[JsonValue] = []
        self.logs: list[JsonValue] = []
        self.fail: list[str] = []
        self.values: dict[str, JsonValue] = {}
        target = call["target"]
        cached_org = call.get("cached_org")
        suffix = ":with_budget" if call.get("include_budget") else ""
        seeded = {
            "org_object": [(f"org_id:{target}{suffix}", cached_org)],
            "org_alias": [(f"org_alias:{target}", cached_org)],
            "org_for_request": [
                (f"org_id:{target}:with_budget", cached_org),
                (f"org_id:{target}:with_budget:last_known", call.get("cached_last_known")),
            ],
            "project_object": [(f"project_id:{target}", call.get("cached_project"))],
        }
        for key, value in seeded[call["function"]]:
            if value is not None:
                self.values[key] = value

    def begin(self, step: Step) -> None:
        self.events = []
        self.logs = []
        self.fail = list(step["fail"])
        time.time = partial(float, step["now"])

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        self.events.append(f"read {key}")
        return self.values.get(key)


# DualCache swallows a failing in-memory write, so a scripted write failure is raised above it
class KeyCache(UserApiKeyCache):
    def __init__(self, fixture: Fixture) -> None:
        super().__init__(in_memory_cache=fixture, default_in_memory_ttl=60.0)
        self.fixture = fixture

    @override
    async def async_set_cache(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/user_api_key_cache.py#L215-L233
        self, key: str, value: BaseModel, *, model_type: type[BaseModel], ttl: float
    ) -> None:
        del model_type
        if f"write {key}" in self.fixture.fail:
            raise RuntimeError(key)
        self.fixture.events.append(f"write {key} ttl={ttl:g}")
        self.fixture.values[key] = JSON.validate_json(value.model_dump_json())


class StoredRow(BaseModel):
    model_config = ConfigDict(extra="allow")


# the route exception's typed fields, since HTTPException.detail carries Any
class Rejection(BaseModel):
    status_code: int
    detail: str | dict[str, str]


class Table:
    def __init__(self, fixture: Fixture, name: str, rows: list[Row]) -> None:
        self.fixture = fixture
        self.name = name
        self.rows = rows

    async def find_unique(
        self, where: dict[str, str], include: dict[str, bool] | None = None
    ) -> StoredRow | None:
        self.refuse()
        field, value = next(iter(where.items()))
        flag = "true" if include else "false"
        self.fixture.events.append(f"find_unique {self.name} {value} include={flag}")
        found = [row for row in self.rows if row.get(field) == value]
        return StoredRow.model_validate(self.visible(found[0], include)) if found else None

    async def find_many(
        self, where: dict[str, str], include: dict[str, bool] | None = None
    ) -> list[StoredRow]:
        self.refuse()
        field, value = next(iter(where.items()))
        self.fixture.events.append(f"find_many {self.name} {value}")
        matched = [self.visible(row, include) for row in self.rows if row.get(field) == value]
        return [StoredRow.model_validate(row) for row in matched]

    def refuse(self) -> None:
        if "outage" in self.fixture.fail:
            raise httpx.ConnectError("the authentication database is unreachable")
        if "fault" in self.fixture.fail:
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
        if name in self.tables:
            return self.tables[name]
        raise AttributeError(name)


class FakePrismaClient(PrismaClient):
    def __init__(self, fixture: Fixture, call: Call) -> None:
        org_row = call.get("org_row")
        rows: list[Row] = [] if org_row is None else [org_row]
        alias_rows = call.get("alias_rows")
        if alias_rows is not None:
            rows.extend(alias_rows)
        project_row = call.get("project_row")
        projects: list[Row] = [] if project_row is None else [project_row]
        tables = {
            "litellm_organizationtable": Table(fixture, "organization", rows),
            "litellm_projecttable": Table(fixture, "project", projects),
        }
        self.db = FakeDb(tables)


class Board(logging.Handler):
    def __init__(self, fixture: Fixture) -> None:
        super().__init__(logging.DEBUG)
        self.fixture: Final = fixture
        self.service_logging_obj: Final = self

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.funcName in LOGGING_FUNCTIONS:
            self.fixture.logs.append(record.levelname)

    async def async_service_success_hook(self, **_kwargs: JsonValue) -> None: ...

    async def async_service_failure_hook(self, **_kwargs: JsonValue) -> None: ...


class ProxyServer(ModuleType):
    general_settings: dict[str, JsonValue]
    proxy_logging_obj: Board


async def looked_up(call: Call, db: FakePrismaClient | None, cache: UserApiKeyCache) -> JsonValue:
    target = call["target"]
    match call["function"]:
        case "org_object":
            org: LiteLLM_OrganizationTable | None = await get_org_object(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L4107-L4176
                org_id=target,
                prisma_client=db,
                user_api_key_cache=cache,
                include_budget_table=bool(call.get("include_budget")),
            )
            return None if org is None else org_view(org)
        case "org_alias":
            alias: LiteLLM_OrganizationTable | None = await get_org_object_by_alias(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3530-L3614
                target, db, cache
            )
            return None if alias is None else org_view(alias)
        case "org_for_request":
            if db is None:
                raise ValueError("the request lookup runs only with a database")
            requested = await get_org_object_for_request(target, db, cache, None, None)
            return None if requested is None else org_view(requested)
        case "project_object":
            project = await get_project_object(target, db, cache)
            return None if project is None else project_view(project)


async def performed(call: Call, db: FakePrismaClient | None, cache: UserApiKeyCache) -> Answer:
    try:
        return await looked_up(call, db, cache), None
    except OrganizationNotFoundError:
        return None, {"kind": "org_not_found", "status": None, "detail": None}
    except HTTPException as raised:
        rejected = TypeAdapter(Rejection).validate_python(raised, from_attributes=True)
        match rejected.detail:
            case str():
                return None, {"kind": "http", "status": rejected.status_code, "detail": "text"}
            case dict():
                return None, {"kind": "http", "status": rejected.status_code, "detail": "object"}
    except (httpx.ConnectError, RuntimeError):
        return None, {"kind": "exception", "status": None, "detail": None}


async def answer(call: Call, fixture: Fixture) -> JsonValue:
    logger = logging.getLogger("LiteLLM Proxy")
    board = Board(fixture)
    logger.setLevel(logging.DEBUG)
    logger.handlers = [board]
    cache = KeyCache(fixture)
    client = FakePrismaClient(fixture, call)
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.proxy_logging_obj = board
    sys.modules[PROXY_MODULE] = proxy_server
    steps: list[JsonValue] = []
    for step in call["steps"]:
        fixture.begin(step)
        allow = step.get("allow")
        allowed = False if allow is None else allow
        proxy_server.general_settings = {"allow_requests_on_db_unavailable": allowed}
        value, error = await performed(call, client if call["database"] else None, cache)
        events = list(fixture.events)
        logs = list(fixture.logs)
        steps.append({"value": value, "error": error, "events": events, "logs": logs})
    return {"steps": steps}


def run(input_value: Call) -> JsonValue:
    if input_value["in_memory_ttl"] != DEFAULT_IN_MEMORY_TTL:
        raise ValueError("in_memory_ttl must equal the DEFAULT_IN_MEMORY_TTL the case env sets")
    return asyncio.run(answer(input_value, Fixture(input_value)))
