import asyncio
from collections.abc import Iterator, Mapping
from typing import Final, Literal, NotRequired, TypedDict

from fastapi import FastAPI
from litellm.proxy._types import LiteLLM_JWTAuth
from litellm.proxy.auth.handle_jwt import AgentLookup, JWTAuthManager, JWTHandler
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.pass_through_endpoints.pass_through_endpoints import (
    InitPassThroughEndpointHelpers,
)
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.types.agents import AgentResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)
JsonRow = dict[str, JsonValue]
Function = Literal["agent", "candidates", "header", "object", "specific_team"]
UNAUTHORIZED: Final = 401
ENDPOINT: Final = "p1"


class Scenario(TypedDict):
    function: Function
    token: NotRequired[JsonRow]
    fields: NotRequired[JsonRow]
    teams: NotRequired[dict[str, JsonRow]]
    aliases: NotRequired[dict[str, list[JsonRow]]]
    agents: NotRequired[list[JsonRow]]
    allowed_team_ids: NotRequired[list[str]]
    headers: NotRequired[dict[str, str]]
    requested_model: NotRequired[str | None]
    route: NotRequired[str]
    request_method: NotRequired[str]
    registered_route: NotRequired[bool]
    user_id: NotRequired[str | None]
    team_id: NotRequired[str | None]
    enforce_rbac: NotRequired[bool]
    is_proxy_admin: NotRequired[bool]
    database: NotRequired[bool]


class Input(TypedDict):
    scenarios: list[Scenario]


# a team row as LiteLLM's own pydantic model reads it; extra table columns are allowed
class Row(BaseModel):
    model_config = ConfigDict(extra="allow")


# the team table the scripted scenario answers from; every read is an event, as on the Rust side
class TeamTable:
    def __init__(
        self,
        teams: dict[str, JsonRow] | None,
        aliases: dict[str, list[JsonRow]] | None,
        events: list[str],
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
            row = None if self.teams is None else self.teams.get(name)
            return [] if row is None else [Row.model_validate(row)]
        if field == "team_alias":
            self.events.append(f"alias {name}")
            rows = None if self.aliases is None else self.aliases.get(name)
            return [] if rows is None else [Row.model_validate(row) for row in rows]
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


class Registry(AgentLookup):
    def __init__(self, rows: list[JsonRow]) -> None:
        self.agents = [AgentResponse.model_validate(row) for row in rows]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/agents.py#L323

    @override
    def get_agent_list(self) -> list[AgentResponse]:
        return list(self.agents)

    @override
    def get_agent_by_id(self, agent_id: str) -> AgentResponse | None:
        return next((agent for agent in self.agents if agent.agent_id == agent_id), None)

    @override
    def get_agent_by_name(self, agent_name: str) -> AgentResponse | None:
        return next((agent for agent in self.agents if agent.agent_name == agent_name), None)


# LiteLLM scans a set in per-process order; both sides read the sorted order the Rust side uses
class ScanOrdered(set[str]):
    @override
    def __iter__(self) -> Iterator[str]:
        return iter(sorted(super().__iter__()))

    @override
    def __repr__(self) -> str:
        members = ", ".join(repr(member) for member in sorted(super().__iter__()))
        return "set()" if not members else f"{{{members}}}"


def token_of(scenario: Scenario) -> JsonRow:
    given = scenario.get("token")
    return {} if given is None else given


def fields_of(scenario: Scenario) -> JsonRow:
    given = scenario.get("fields")
    return {} if given is None else given


def agents_of(scenario: Scenario) -> list[JsonRow]:
    given = scenario.get("agents")
    return [] if given is None else given


def route_of(scenario: Scenario) -> str:
    given = scenario.get("route")
    return "" if given is None else given


def allowed_teams_of(scenario: Scenario) -> set[str]:
    given = scenario.get("allowed_team_ids")
    return set[str]() if given is None else set(given)


def handler_of(
    scenario: Scenario, events: list[str], cache: UserApiKeyCache
) -> tuple[Client | None, JWTHandler]:
    client = (
        None
        if scenario.get("database") is False
        else Client(TeamTable(scenario.get("teams"), scenario.get("aliases"), events))
    )
    handler = JWTHandler()
    handler.update_environment(
        prisma_client=client,
        user_api_key_cache=cache,
        litellm_jwtauth=LiteLLM_JWTAuth(**fields_of(scenario)),
    )
    return client, handler


def register_route(scenario: Scenario) -> None:
    InitPassThroughEndpointHelpers.clear_all_pass_through_routes()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/pass_through_endpoints.py#L3145
    if scenario.get("registered_route") is not True:
        return
    method = scenario.get("request_method")
    route = scenario.get("route")
    InitPassThroughEndpointHelpers.add_exact_path_route(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/pass_through_endpoints.py#L2957
        app=FastAPI(),
        path="" if route is None else route,
        target="https://oracle.invalid",
        custom_headers=None,
        forward_headers=None,
        merge_query_params=None,
        dependencies=None,
        cost_per_request=None,
        endpoint_id=ENDPOINT,
        methods=["" if method is None else method.upper()],
        auth=True,
    )


# B757's public shape for an auth failure: type, param and the status as the code string
def failure(status: int, message: str, events: list[str]) -> JsonValue:
    error_row: JsonRow = {
        "status": status,
        "message": message,
        "type": "auth_error",
        "param": "None",
        "code": str(status),
    }
    payload: JsonRow = {"error": error_row, "events": JSON.validate_python(events)}
    return JSON.validate_python(payload)


async def answered(scenario: Scenario) -> JsonValue:
    events: list[str] = []
    cache = UserApiKeyCache()
    client, handler = handler_of(scenario, events, cache)
    token = token_of(scenario)
    function = scenario["function"]
    register_route(scenario)
    value: JsonValue = None
    row: JsonRow = {}
    try:
        if function == "agent":
            agent_id = JWTAuthManager.resolve_agent_id(
                handler,
                token,
                Registry(agents_of(scenario)),
            )
            row = {"agent_id": agent_id}
            value = JSON.validate_python(row)
        elif function == "specific_team":
            team_id, _team = await JWTAuthManager.find_and_validate_specific_team_id(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L1523-L1620
                jwt_handler=handler,
                jwt_valid_token=token,
                prisma_client=client,
                user_api_key_cache=cache,
                parent_otel_span=None,
                proxy_logging_obj=ProxyLogging(user_api_key_cache=cache),
            )
            row = {"team_id": team_id}
            value = JSON.validate_python(row)
        elif function == "candidates":
            team_ids = ScanOrdered(JWTAuthManager.get_all_team_ids(handler, token))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L1623-L1629
            selected, _team = await JWTAuthManager.find_team_with_model_access(
                team_ids=team_ids,
                requested_model=scenario.get("requested_model"),
                route=route_of(scenario),
                jwt_handler=handler,
                prisma_client=client,
                user_api_key_cache=cache,
                parent_otel_span=None,
                proxy_logging_obj=ProxyLogging(user_api_key_cache=cache),
                request_method=scenario.get("request_method"),
            )
            row = {"team_id": selected}
            value = JSON.validate_python(row)
        elif function == "header":
            header = await JWTAuthManager.resolve_team_from_header(
                request_headers=scenario.get("headers"),
                allowed_team_ids=allowed_teams_of(scenario),
                fallback_to_db_teams=fields_of(scenario).get("fallback_to_db_teams") is True,
                prisma_client=client,
                user_api_key_cache=cache,
                parent_otel_span=None,
                proxy_logging_obj=ProxyLogging(user_api_key_cache=cache),
            )
            row = {"header_value": None, "team_id": None}
            if header is not None:
                row = {"header_value": header.header_value, "team_id": header.team_id}
            value = JSON.validate_python(row)
        else:
            JWTAuthManager.validate_object_id(
                scenario.get("user_id"),
                scenario.get("team_id"),
                scenario.get("enforce_rbac") is True,
                scenario.get("is_proxy_admin") is True,
            )
            row = {"ok": True}
            value = JSON.validate_python(row)
    except HTTPException as error:
        return failure(error.status_code, str(error.detail), events)
    except Exception as error:
        if not str(error).startswith(("No DB Connected", "No team found in token")):
            raise
        return failure(UNAUTHORIZED, f"Authentication Error, {error}", events)
    events = sorted(events) if function == "candidates" else events
    row = {"events": JSON.validate_python(events), "value": value}
    return JSON.validate_python(row)


async def answered_all(input_value: Input) -> JsonValue:
    answers = [await answered(scenario) for scenario in input_value["scenarios"]]
    result: JsonRow = {"scenarios": answers}
    return JSON.validate_python(result)


def run(input_value: Input) -> JsonValue:
    answers = asyncio.run(answered_all(input_value))
    return JSON.validate_python(answers)
