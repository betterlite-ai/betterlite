import asyncio
from collections.abc import Iterator, Mapping
from typing import Final, Literal, NotRequired, TypedDict

from fastapi import FastAPI, HTTPException
from litellm.proxy._types import LiteLLM_JWTAuth
from litellm.proxy.auth.handle_jwt import JWTAuthManager, JWTHandler
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.pass_through_endpoints.pass_through_endpoints import (
    InitPassThroughEndpointHelpers,
)
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.types.agents import AgentResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)
JsonRow = dict[str, JsonValue]
Function = Literal["agent", "candidates", "header", "object", "specific_team"]
UNAUTHORIZED: Final = 401
ENDPOINT: Final = "p1"


class Fields(TypedDict, total=False):
    team_id_jwt_field: str | None
    team_ids_jwt_field: str | None
    team_id_default: str | None
    team_alias_jwt_field: str | None
    agent_id_jwt_field: str | None
    team_id_upsert: bool
    team_claim_fallback: bool
    fallback_to_db_teams: bool
    enforce_team_based_model_access: bool
    team_allowed_routes: list[str]


class Scenario(TypedDict):
    function: Function
    token: NotRequired[JsonRow]
    fields: NotRequired[Fields]
    teams: NotRequired[dict[str, JsonRow]]
    aliases: NotRequired[dict[str, list[JsonRow]]]
    agents: NotRequired[list[JsonRow]]
    allowed_team_ids: NotRequired[list[str]]
    headers: NotRequired[dict[str, str]]
    requested_model: NotRequired[str | None]
    route: NotRequired[str]
    request_method: NotRequired[str | None]
    registered_route: NotRequired[bool]
    user_id: NotRequired[str | None]
    team_id: NotRequired[str | None]
    enforce_rbac: NotRequired[bool]
    is_proxy_admin: NotRequired[bool]
    database: NotRequired[bool]


class Input(TypedDict):
    scenarios: list[Scenario]


# a team row as LiteLLM's own pydantic model reads it; extra fields are allowed like the table's columns
class Row(BaseModel):
    model_config = ConfigDict(extra="allow")


# the team table the scripted scenario answers from; every read is an event, as on the Rust side
class TeamTable:
    def __init__(
        self, teams: dict[str, JsonRow], aliases: dict[str, list[JsonRow]], events: list[str]
    ) -> None:
        self.teams = teams
        self.aliases = aliases
        self.events = events

    async def find_unique(
        self, *, where: Mapping[str, JsonValue], include: Mapping[str, JsonValue] | None = None
    ) -> Row | None:
        found = await self.find_many(where=where, include=include)
        return found.pop() if found else None

    async def find_many(
        self, *, where: Mapping[str, JsonValue], include: Mapping[str, JsonValue] | None = None
    ) -> list[Row]:
        del include
        field, value = next(iter(where.items()))
        name = TEXT.validate_python(value)
        if field == "team_id":
            self.events.append(f"team {name}")
            row = self.teams.get(name)
            return [] if row is None else [Row.model_validate(row)]
        if field == "team_alias":
            self.events.append(f"alias {name}")
            return [Row.model_validate(row) for row in self.aliases.get(name, [])]
        raise ValueError(f"unexpected team read on {field}")


class Db(PrismaWrapper):
    def __init__(self, table: TeamTable) -> None:
        self.table = table

    @override
    def __getattr__(self, name: str) -> TeamTable:
        if name != "litellm_teamtable":
            raise AttributeError(name)
        return self.table


class Client(PrismaClient):
    def __init__(self, table: TeamTable) -> None:
        self.db = Db(table)


class Registry:
    def __init__(self, rows: list[JsonRow]) -> None:
        self.agents = [AgentResponse.model_validate(row) for row in rows]

    def get_agent_list(self) -> list[AgentResponse]:
        return list(self.agents)

    def get_agent_by_id(self, agent_id: str) -> AgentResponse | None:
        return next((agent for agent in self.agents if agent.agent_id == agent_id), None)

    def get_agent_by_name(self, agent_name: str) -> AgentResponse | None:
        return next((agent for agent in self.agents if agent.agent_name == agent_name), None)


# LiteLLM scans a set, whose iteration order differs per process; the sorted order is the one the
# Rust side uses, so the scan's reads line up. No case has two teams that both qualify.
class ScanOrdered(set[str]):
    @override
    def __iter__(self) -> Iterator[str]:
        return iter(sorted(super().__iter__()))

    @override
    def __repr__(self) -> str:
        members = ", ".join(repr(member) for member in sorted(super().__iter__()))
        return "set()" if not members else f"{{{members}}}"


def token_of(scenario: Scenario) -> JsonRow:
    return scenario.get("token", {})


def fields_of(scenario: Scenario) -> Fields:
    return scenario.get("fields", Fields())


def handler_of(scenario: Scenario, client: Client | None, cache: UserApiKeyCache) -> JWTHandler:
    handler = JWTHandler()
    handler.update_environment(
        prisma_client=client,
        user_api_key_cache=cache,
        litellm_jwtauth=LiteLLM_JWTAuth(**fields_of(scenario)),
    )
    return handler


def register_route(scenario: Scenario) -> None:
    InitPassThroughEndpointHelpers.clear_all_pass_through_routes()
    if not scenario.get("registered_route", False):
        return
    method = (scenario.get("request_method") or "").upper()
    InitPassThroughEndpointHelpers.add_exact_path_route(
        app=FastAPI(),
        path=scenario.get("route", ""),
        target="https://oracle.invalid",
        custom_headers=None,
        forward_headers=None,
        merge_query_params=None,
        dependencies=None,
        cost_per_request=None,
        endpoint_id=ENDPOINT,
        methods=[method],
        auth=True,
    )


# the owned code raises these two as a plain Exception, which B757 renders as a 401; anything else
# is a collaborator failure the case must show instead of hiding
def owned_plain_failure(error: Exception) -> bool:
    return str(error).startswith(("No DB Connected", "No team found in token"))


def failure(error: Exception, events: list[str]) -> JsonValue:
    if isinstance(error, HTTPException):
        status = error.status_code
        message = str(error.detail)
    else:
        status = UNAUTHORIZED
        message = f"Authentication Error, {error}"
    # B757's public shape for an auth failure: type, param and the status as the code string
    return JSON.validate_python(
        {
            "error": {
                "status": status,
                "message": message,
                "type": "auth_error",
                "param": "None",
                "code": str(status),
            },
            "events": events,
        }
    )


async def answered(scenario: Scenario) -> JsonValue:
    events: list[str] = []
    cache = UserApiKeyCache()
    table = TeamTable(scenario.get("teams", {}), scenario.get("aliases", {}), events)
    client = Client(table) if scenario.get("database", True) else None
    handler = handler_of(scenario, client, cache)
    token = token_of(scenario)
    function = scenario["function"]
    register_route(scenario)
    value: JsonValue = None
    try:
        if function == "agent":
            agent_id = JWTAuthManager.resolve_agent_id(
                handler, token, Registry(scenario.get("agents", []))
            )
            value = JSON.validate_python({"agent_id": agent_id})
        elif function == "specific_team":
            team_id, _team = await JWTAuthManager.find_and_validate_specific_team_id(
                jwt_handler=handler,
                jwt_valid_token=token,
                prisma_client=client,
                user_api_key_cache=cache,
                parent_otel_span=None,
                proxy_logging_obj=ProxyLogging(user_api_key_cache=cache),
            )
            value = JSON.validate_python({"team_id": team_id})
        elif function == "candidates":
            team_ids = ScanOrdered(JWTAuthManager.get_all_team_ids(handler, token))
            selected, _team = await JWTAuthManager.find_team_with_model_access(
                team_ids=team_ids,
                requested_model=scenario.get("requested_model"),
                route=scenario.get("route", ""),
                jwt_handler=handler,
                prisma_client=client,
                user_api_key_cache=cache,
                parent_otel_span=None,
                proxy_logging_obj=ProxyLogging(user_api_key_cache=cache),
                request_method=scenario.get("request_method"),
            )
            value = JSON.validate_python({"team_id": selected})
        elif function == "header":
            header = await JWTAuthManager.resolve_team_from_header(
                request_headers=scenario.get("headers", {}),
                allowed_team_ids=set(scenario.get("allowed_team_ids", [])),
                fallback_to_db_teams=fields_of(scenario).get("fallback_to_db_teams", False),
                prisma_client=client,
                user_api_key_cache=cache,
                parent_otel_span=None,
                proxy_logging_obj=ProxyLogging(user_api_key_cache=cache),
            )
            value = JSON.validate_python(
                {
                    "header_value": None if header is None else header.header_value,
                    "team_id": None if header is None else header.team_id,
                }
            )
        else:
            JWTAuthManager.validate_object_id(
                scenario.get("user_id"),
                scenario.get("team_id"),
                scenario.get("enforce_rbac", False),
                scenario.get("is_proxy_admin", False),
            )
            value = JSON.validate_python({"ok": True})
    except HTTPException as error:
        return failure(error, events)
    except Exception as error:
        if not owned_plain_failure(error):
            raise
        return failure(error, events)
    return JSON.validate_python(
        {"events": sorted(events) if function == "candidates" else events, "value": value}
    )


async def answered_all(input_value: Input) -> JsonValue:
    answers = [await answered(scenario) for scenario in input_value["scenarios"]]
    return JSON.validate_python({"scenarios": answers})


def run(input_value: Input) -> JsonValue:
    return JSON.validate_python(asyncio.run(answered_all(input_value)))
