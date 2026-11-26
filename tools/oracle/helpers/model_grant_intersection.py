import asyncio
import sys
from collections.abc import Mapping
from types import ModuleType
from typing import Final, Literal, NotRequired, Required, TypedDict

import httpx
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.models.team_membership import LiteLLM_TeamMembership
from litellm.proxy._types import LiteLLM_ProjectTableCachedObj, UserAPIKeyAuth
from litellm.proxy.auth.auth_checks import (
    collect_matched_model_access_groups,
    enforced_model_allowlists,
    stamp_matched_model_access_groups,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.router import Router
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
TEXT_WHERE: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])
AUTH: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)

Function = Literal["enforced", "collected", "stamped"]
Fault = Literal["team", "user", "project", "membership", "org", "serving"]
Kind = Literal["team", "user", "project", "org"]
Row = dict[str, object]


class Token(TypedDict, total=False):
    models: list[str]
    team_models: list[str]
    team_id: str
    user_id: str
    project_id: str
    org_id: str


class TeamRow(TypedDict, total=False):
    team_id: Required[str]
    models: list[str]
    organization_id: str


class UserRow(TypedDict, total=False):
    user_id: Required[str]
    models: list[str]


class ProjectRow(TypedDict, total=False):
    project_id: Required[str]
    models: list[str]


class MembershipRow(TypedDict, total=False):
    user_id: Required[str]
    team_id: Required[str]
    allowed_models: list[str]


class OrgRow(TypedDict, total=False):
    organization_id: Required[str]
    models: list[str]


class Call(TypedDict):
    function: Function
    database: bool
    token: Token
    models: NotRequired[list[str]]
    membership_loaded: NotRequired[bool]
    stamp_before: NotRequired[list[str]]
    team: NotRequired[TeamRow]
    user: NotRequired[UserRow]
    project: NotRequired[ProjectRow]
    membership: NotRequired[MembershipRow]
    org: NotRequired[OrgRow]
    registry: NotRequired[list[str]]
    serving: NotRequired[dict[str, list[str]]]
    outages: NotRequired[list[Fault]]


class Rejection(BaseModel):
    status_code: int
    detail: str | dict[str, str]


class StoredRow(BaseModel):
    model_config = ConfigDict(extra="allow")


class Harness:
    def __init__(self) -> None:
        self.events: list[str] = []
        self.cache: UserApiKeyCache = UserApiKeyCache()
        self.proxy: ProxyLogging = ProxyLogging(user_api_key_cache=self.cache)


class ProxyServer(ModuleType):
    proxy_logging_obj: ProxyLogging


class KeyedTable:
    def __init__(
        self, kind: Kind, rows: dict[str, StoredRow], events: list[str], *, outage: bool
    ) -> None:
        self.kind = kind
        self.rows = rows
        self.events = events
        self.outage = outage

    async def find_unique(
        self, *, where: Mapping[str, object], include: Mapping[str, object] | None = None
    ) -> StoredRow | None:
        del include
        identifier = next(iter(TEXT_WHERE.validate_python(where).values()))
        self.events.append(f"read {self.kind} {identifier}")
        if self.outage:
            raise httpx.ConnectError(f"the {self.kind} table is down")
        return self.rows.get(identifier)


class MembershipTable:
    def __init__(
        self, rows: dict[tuple[str, str], StoredRow], events: list[str], *, outage: bool
    ) -> None:
        self.rows = rows
        self.events = events
        self.outage = outage

    async def find_unique(
        self, *, where: Mapping[str, object], include: Mapping[str, object] | None = None
    ) -> StoredRow | None:
        del include
        keyed = TEXT_WHERE.validate_python(where["user_id_team_id"])
        self.events.append(f"read membership {keyed['user_id']} {keyed['team_id']}")
        if self.outage:
            raise httpx.ConnectError("the membership table is down")
        return self.rows.get((keyed["user_id"], keyed["team_id"]))


class RegistryTable:
    def __init__(self, names: list[str] | None, events: list[str]) -> None:
        self.names = names
        self.events = events

    async def find_many(
        self,
        *,
        where: Mapping[str, object] | None = None,
        include: Mapping[str, object] | None = None,
        take: int | None = None,
    ) -> list[StoredRow]:
        del where, include, take
        self.events.append("read registry")
        if self.names is None:
            raise httpx.ConnectError("the registry table is down")
        return [StoredRow.model_validate({"access_group_name": name}) for name in self.names]


Table = KeyedTable | MembershipTable | RegistryTable


class FakeDb(PrismaWrapper):
    def __init__(self, tables: dict[str, Table]) -> None:
        self.tables = tables

    @override
    def __getattr__(self, name: str) -> Table:
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
        groups = self.oracle_serving.get(model_name if model_name is not None else "", [])
        return {group: [] for group in groups}


def team_payload(team: TeamRow) -> Row:
    row: Row = {"team_id": team["team_id"], "models": team.get("models") or []}
    organization_id = team.get("organization_id")
    if organization_id is not None:
        row["organization_id"] = organization_id
    return row


def user_payload(user: UserRow) -> Row:
    return {
        "user_id": user["user_id"],
        "models": user.get("models") or [],
        "organization_memberships": None,
    }


def project_payload(project: ProjectRow) -> Row:
    return {"project_id": project["project_id"], "models": project.get("models") or []}


def membership_payload(membership: MembershipRow) -> Row:
    row: Row = {"user_id": membership["user_id"], "team_id": membership["team_id"]}
    allowed = membership.get("allowed_models")
    if allowed is not None:
        row["litellm_budget_table"] = {"allowed_models": allowed}
    return row


def org_payload(org: OrgRow) -> Row:
    return {
        "organization_id": org["organization_id"],
        "budget_id": "oracle-budget",
        "created_by": "oracle",
        "updated_by": "oracle",
        "models": org.get("models") or [],
    }


def client_of(call: Call, events: list[str]) -> FakePrismaClient:
    outages = call.get("outages") or []
    tables: dict[str, Table] = {}
    teams: dict[str, StoredRow] = {}
    team = call.get("team")
    if team is not None:
        teams[team["team_id"]] = StoredRow.model_validate(team_payload(team))
    tables["litellm_teamtable"] = KeyedTable("team", teams, events, outage="team" in outages)
    users: dict[str, StoredRow] = {}
    user = call.get("user")
    if user is not None:
        users[user["user_id"]] = StoredRow.model_validate(user_payload(user))
    tables["litellm_usertable"] = KeyedTable("user", users, events, outage="user" in outages)
    projects: dict[str, StoredRow] = {}
    project = call.get("project")
    if project is not None:
        projects[project["project_id"]] = StoredRow.model_validate(project_payload(project))
    tables["litellm_projecttable"] = KeyedTable(
        "project", projects, events, outage="project" in outages
    )
    orgs: dict[str, StoredRow] = {}
    org = call.get("org")
    if org is not None:
        orgs[org["organization_id"]] = StoredRow.model_validate(org_payload(org))
    tables["litellm_organizationtable"] = KeyedTable("org", orgs, events, outage="org" in outages)
    memberships: dict[tuple[str, str], StoredRow] = {}
    membership = call.get("membership")
    if membership is not None:
        key = (membership["user_id"], membership["team_id"])
        memberships[key] = StoredRow.model_validate(membership_payload(membership))
    tables["litellm_teammembership"] = MembershipTable(
        memberships, events, outage="membership" in outages
    )
    tables["litellm_modelaccessgroupbudgettable"] = RegistryTable(call.get("registry"), events)
    return FakePrismaClient(FakeDb(tables))


def auth_of(call: Call) -> UserAPIKeyAuth:
    token = call["token"]
    payload: Row = {
        "models": token.get("models") or [],
        "team_models": token.get("team_models") or [],
        "team_id": token.get("team_id"),
        "user_id": token.get("user_id"),
        "project_id": token.get("project_id"),
        "org_id": token.get("org_id"),
    }
    stamp = call.get("stamp_before")
    if stamp is not None:
        payload["matched_model_access_groups"] = stamp
    return AUTH.validate_python(payload)


def team_object(call: Call) -> LiteLLM_TeamTableCachedObj | None:
    team = call.get("team")
    if team is None:
        return None
    return LiteLLM_TeamTableCachedObj.model_validate(team_payload(team))


def project_object(call: Call) -> LiteLLM_ProjectTableCachedObj | None:
    project = call.get("project")
    if project is None:
        return None
    return LiteLLM_ProjectTableCachedObj.model_validate(project_payload(project))


def membership_object(call: Call) -> LiteLLM_TeamMembership | None:
    membership = call.get("membership")
    if membership is None:
        return None
    return LiteLLM_TeamMembership.model_validate(membership_payload(membership))


def router_of(call: Call, harness: Harness) -> ScriptedRouter:
    outages = call.get("outages") or []
    return ScriptedRouter(
        call.get("serving") or {}, harness.events, unreadable="serving" in outages
    )


async def enforced(call: Call, auth: UserAPIKeyAuth, harness: Harness) -> JsonValue:
    client = client_of(call, harness.events) if call["database"] else None
    allowlists = await enforced_model_allowlists(
        valid_token=auth,
        prisma_client=client,
        user_api_key_cache=harness.cache,
        proxy_logging_obj=harness.proxy,
    )
    return JSON.validate_python([[str(model) for model in models] for models in allowlists])


async def collected(call: Call, auth: UserAPIKeyAuth, harness: Harness) -> JsonValue:
    client = client_of(call, harness.events) if call["database"] else None
    matched = await collect_matched_model_access_groups(
        model=call.get("models"),
        valid_token=auth,
        team_object=team_object(call),
        project_object=project_object(call),
        llm_router=router_of(call, harness),
        prisma_client=client,
        user_api_key_cache=harness.cache,
        proxy_logging_obj=harness.proxy,
        team_membership=membership_object(call),
        team_membership_loaded=call.get("membership_loaded", False),
    )
    return JSON.validate_python([str(name) for name in matched])


async def stamped(call: Call, auth: UserAPIKeyAuth, harness: Harness) -> JsonValue:
    client = client_of(call, harness.events) if call["database"] else None
    returned = await stamp_matched_model_access_groups(
        model=call.get("models"),
        valid_token=auth,
        team_object=team_object(call),
        project_object=project_object(call),
        llm_router=router_of(call, harness),
        prisma_client=client,
        user_api_key_cache=harness.cache,
        proxy_logging_obj=harness.proxy,
        team_membership=membership_object(call),
        team_membership_loaded=call.get("membership_loaded", False),
    )
    stored = [str(name) for name in (auth.matched_model_access_groups or [])]
    return JSON.validate_python(
        {"returned": [str(name) for name in returned], "stored": stored}
    )


async def answered(call: Call, harness: Harness) -> JsonValue:
    auth = auth_of(call)
    match call["function"]:
        case "enforced":
            return await enforced(call, auth, harness)
        case "collected":
            return await collected(call, auth, harness)
        case "stamped":
            return await stamped(call, auth, harness)


async def performed(call: Call, harness: Harness) -> JsonValue:
    try:
        value = await answered(call, harness)
    except HTTPException as raised:
        rejection = TypeAdapter(Rejection).validate_python(raised, from_attributes=True)
        return JSON.validate_python(
            {"value": None, "error": {"status": rejection.status_code}, "events": harness.events}
        )
    except (httpx.ConnectError, RuntimeError, ValueError):
        return JSON.validate_python(
            {"value": None, "error": {"status": None}, "events": harness.events}
        )
    return JSON.validate_python({"value": value, "error": None, "events": harness.events})


async def answer(call: Call, harness: Harness) -> JsonValue:
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.proxy_logging_obj = harness.proxy
    sys.modules[PROXY_MODULE] = proxy_server
    return await performed(call, harness)


def run(input_value: Call) -> JsonValue:
    return asyncio.run(answer(input_value, Harness()))
