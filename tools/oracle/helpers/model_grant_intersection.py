import asyncio
from collections.abc import Mapping
from typing import Final, Literal, NotRequired, TypedDict

import httpx
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.models.team_membership import LiteLLM_TeamMembership
from litellm.proxy._types import LiteLLM_ProjectTableCachedObj, UserAPIKeyAuth
from litellm.proxy.auth.auth_checks import (
    collect_matched_model_access_groups,
    enforced_model_allowlists,
    get_project_object,
    get_team_membership,
    get_team_object,
    stamp_matched_model_access_groups,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.router import Router
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
LISTS: Final[TypeAdapter[list[str]]] = TypeAdapter(list[str])
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)
KEYED: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])
AUTH: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)
Function = Literal["enforced", "collected", "stamped"]
Fault = Literal["team", "user", "project", "membership", "org", "serving"]
JsonRow = dict[str, JsonValue]
Row = dict[str, object]


class Call(TypedDict):
    function: Function
    database: bool
    token: JsonRow
    models: NotRequired[list[str]]
    membership_loaded: NotRequired[bool]
    stamp_before: NotRequired[list[str]]
    team: NotRequired[JsonRow]
    user: NotRequired[JsonRow]
    project: NotRequired[JsonRow]
    membership: NotRequired[JsonRow]
    org: NotRequired[JsonRow]
    registry: NotRequired[list[str]]
    serving: NotRequired[dict[str, list[str]]]
    outages: NotRequired[list[Fault]]


class StoredRow(BaseModel):
    model_config = ConfigDict(extra="allow")
    budget_id: str = "oracle-budget"
    created_by: str = "oracle"
    updated_by: str = "oracle"
    organization_memberships: JsonValue = None


class Harness:
    def __init__(self) -> None:
        self.events: list[str] = []
        self.cache: UserApiKeyCache = UserApiKeyCache()
        self.proxy: ProxyLogging = ProxyLogging(user_api_key_cache=self.cache)


class Table:
    def __init__(
        self, kind: str, rows: dict[tuple[str, ...], StoredRow], events: list[str], *, outage: bool
    ) -> None:
        self.kind = kind
        self.rows = rows
        self.events = events
        self.outage = outage

    async def find_unique(
        self, *, where: Mapping[str, JsonValue], include: Mapping[str, JsonValue] | None = None
    ) -> StoredRow | None:
        del include
        key = self.key(where)
        self.events.append(f"read {self.kind} {' '.join(key)}")
        if self.outage:
            raise httpx.ConnectError(f"the {self.kind} table is down")
        return self.rows.get(key)

    def key(self, where: Mapping[str, JsonValue]) -> tuple[str, ...]:
        values = list(where.values())
        if self.kind != "membership":
            return (TEXT.validate_python(values[0]),)
        keyed = KEYED.validate_python(values[0])
        return (keyed["user_id"], keyed["team_id"])


class RegistryTable:
    def __init__(self, names: list[str] | None, events: list[str]) -> None:
        self.names = names
        self.events = events

    async def find_many(self, *, take: int) -> list[StoredRow]:
        del take
        self.events.append("read registry")
        names = self.names
        if names is None:
            raise httpx.ConnectError("the registry table is down")
        rows: list[StoredRow] = []
        for name in names:
            payload: Row = {"access_group_name": name}
            rows.append(StoredRow.model_validate(payload))
        return rows


class FakeDb(PrismaWrapper):
    def __init__(self, tables: dict[str, Table | RegistryTable]) -> None:
        self.tables = tables

    @override
    def __getattr__(self, name: str) -> Table | RegistryTable:
        table = self.tables.get(name)
        if table is None:
            raise AttributeError(name)
        return table


class FakePrismaClient(PrismaClient):
    def __init__(self, db: FakeDb) -> None:
        self.db = db


class ScriptedRouter(Router):
    def __init__(
        self, serving: dict[str, list[str]], events: list[str], *, unreadable: bool
    ) -> None:
        self.oracle_serving = serving
        self.oracle_events = events
        self.oracle_unreadable = unreadable

    @override
    def get_model_access_groups(
        self,
        model_name: str | None = None,
        model_access_group: str | None = None,
        team_id: str | None = None,
    ) -> dict[str, list[str]]:
        del model_access_group
        team = "-" if team_id is None else team_id
        self.oracle_events.append(f"serving {model_name} team={team}")
        if self.oracle_unreadable:
            raise RuntimeError("the access group store is down")
        groups: list[str] = []
        if model_name is not None:
            found = self.oracle_serving.get(model_name)
            if found is not None:
                groups = found
        return {group: [] for group in groups}


def model_list(value: JsonValue) -> list[str]:
    if value is None:
        return []
    return LISTS.validate_python(value)


def outages_of(call: Call) -> list[Fault]:
    listed = call.get("outages")
    if listed is None:
        return []
    return listed


def identifier(row: JsonRow) -> tuple[str, ...]:
    for name in ("team_id", "user_id", "project_id", "organization_id"):
        value = row.get(name)
        if value is not None:
            return (TEXT.validate_python(value),)
    raise ValueError("the row names no identifier")


def client_of(call: Call, events: list[str], *, faults: bool = True) -> FakePrismaClient:
    outages = [] if not faults else outages_of(call)
    db: dict[str, Table | RegistryTable] = {}
    sources: list[tuple[str, str, JsonRow | None]] = [
        ("litellm_teamtable", "team", call.get("team")),
        ("litellm_usertable", "user", call.get("user")),
        ("litellm_projecttable", "project", call.get("project")),
        ("litellm_organizationtable", "org", call.get("org")),
    ]
    for name, kind, source in sources:
        rows: dict[tuple[str, ...], StoredRow] = {}
        if source is not None:
            rows[identifier(source)] = StoredRow.model_validate(source)
        db[name] = Table(kind, rows, events, outage=kind in outages)
    membership = call.get("membership")
    member_rows: dict[tuple[str, ...], StoredRow] = {}
    if membership is not None:
        row: Row = {"user_id": membership["user_id"], "team_id": membership["team_id"]}
        allowed = membership.get("allowed_models")
        if allowed is not None:
            row["litellm_budget_table"] = {"allowed_models": allowed}
        member_user = TEXT.validate_python(membership["user_id"])
        member_team = TEXT.validate_python(membership["team_id"])
        member_rows[(member_user, member_team)] = StoredRow.model_validate(row)
    db["litellm_teammembership"] = Table(
        "membership", member_rows, events, outage="membership" in outages
    )
    db["litellm_modelaccessgroupbudgettable"] = RegistryTable(call.get("registry"), events)
    return FakePrismaClient(FakeDb(db))


def auth_of(call: Call) -> UserAPIKeyAuth:
    token = call["token"]
    payload: Row = {
        "models": model_list(token.get("models")),
        "team_models": model_list(token.get("team_models")),
        "team_id": token.get("team_id"),
        "user_id": token.get("user_id"),
        "project_id": token.get("project_id"),
        "org_id": token.get("org_id"),
    }
    stamp = call.get("stamp_before")
    if stamp is not None:
        payload["matched_model_access_groups"] = stamp
    return AUTH.validate_python(payload)


async def walked(call: Call, auth: UserAPIKeyAuth, harness: Harness) -> JsonValue:
    client = client_of(call, harness.events) if call["database"] else None
    if call["function"] == "enforced":
        allowlists = await enforced_model_allowlists(auth, client, harness.cache, harness.proxy)
        return JSON.validate_python([list(models) for models in allowlists])
    loaded = bool(call.get("membership_loaded"))
    preloader = client_of(call, [], faults=False)
    team: LiteLLM_TeamTableCachedObj | None = None
    project: LiteLLM_ProjectTableCachedObj | None = None
    membership: LiteLLM_TeamMembership | None = None
    listed_team = call.get("team")
    if listed_team is not None:
        team_id = TEXT.validate_python(listed_team["team_id"])
        team = await get_team_object(team_id, preloader, harness.cache)
    listed_project = call.get("project")
    if listed_project is not None:
        project_id = TEXT.validate_python(listed_project["project_id"])
        project = await get_project_object(project_id, preloader, harness.cache)
    listed_membership = call.get("membership")
    if listed_membership is not None and loaded:
        user_id = TEXT.validate_python(listed_membership["user_id"])
        member_team = TEXT.validate_python(listed_membership["team_id"])
        membership = await get_team_membership(user_id, member_team, preloader, harness.cache)
    router = router_of(call, harness)
    if call["function"] == "collected":
        matched = await collect_matched_model_access_groups(
            call.get("models"), auth, team, project, router, client,
            harness.cache, harness.proxy, membership, loaded,
        )
        return JSON.validate_python(list(matched))
    returned = await stamp_matched_model_access_groups(
        call.get("models"), auth, team, project, router, client,
        harness.cache, harness.proxy, membership, loaded,
    )
    return stamp_of(returned, auth.matched_model_access_groups)


def router_of(call: Call, harness: Harness) -> ScriptedRouter:
    serving = call.get("serving")
    if serving is None:
        serving = {}
    return ScriptedRouter(serving, harness.events, unreadable="serving" in outages_of(call))


def stamp_of(returned: tuple[str, ...], stored: list[str] | None) -> JsonValue:
    stamp: dict[str, list[str]] = {
        "returned": list(returned),
        "stored": [] if stored is None else stored,
    }
    return JSON.validate_python(stamp)


def run(input_value: Call) -> JsonValue:
    harness = Harness()
    value: JsonValue = None
    error: JsonValue = None
    try:
        value = asyncio.run(walked(input_value, auth_of(input_value), harness))
    except HTTPException as raised:
        failure: dict[str, JsonValue] = {"status": raised.status_code}
        error = JSON.validate_python(failure)
    except (httpx.ConnectError, RuntimeError, ValueError):
        missing: dict[str, JsonValue] = {"status": None}
        error = JSON.validate_python(missing)
    answer: JsonRow = {
        "value": value,
        "error": error,
        "events": JSON.validate_python(harness.events),
    }
    return JSON.validate_python(answer)
