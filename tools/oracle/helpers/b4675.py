import asyncio
import importlib
import json
import os
import secrets
from collections.abc import Sequence
from datetime import UTC, datetime, tzinfo
from typing import ClassVar, Final, Generic, Literal, Self, TypedDict, TypeVar

from litellm.caching.dual_cache import DualCache
from litellm.proxy._experimental.mcp_server import gateway_dcr_flow
from litellm.proxy._experimental.mcp_server.gateway_dcr_flow import (
    ConsentTeam,
    aggregate_authorize,
    complete_connect_flow,
    describe_connect_flow,
    native_client_auth_contract,
    native_client_authorize,
    register_aggregate_client,
)
from litellm.proxy._experimental.mcp_server.mcp_server_manager import global_mcp_server_manager
from litellm.proxy.common_utils.html_forms import native_client_consent
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.responses import Response
from typing_extensions import override

CACHE_HEADERS: Final = ("cache-control", "content-security-policy", "pragma", "x-frame-options")
REDIRECT: Final = 303

Kind = Literal["aggregate", "native", "describe", "complete", "contract"]
VendorState = Literal["present", "absent", "unavailable"]
ReloadFailure = Literal["unresolvable", "unavailable", "faulted", "no_active_key"]
T = TypeVar("T")


class ClientSeed(TypedDict):
    redirect_uris: list[str]


class TeamSeed(TypedDict):
    team_id: str
    team_alias: str | None


class Step(TypedDict):
    name: str
    kind: Kind
    now: int
    path: str
    query: str | None
    cookie_from: int
    client: int
    client_id: str
    redirect_uri: str
    state: str
    code_challenge: str | None
    code_challenge_method: str | None
    response_type: str | None
    user: str | None
    resource: str | None
    tokens: list[str]
    handle: str
    delivery: str | None
    team_id: str | None
    decision: str | None
    teams: list[TeamSeed]
    teams_failure: ReloadFailure | None
    vendor: VendorState
    reachable: bool
    token_exchange: bool


class Input(TypedDict):
    clients: list[ClientSeed]
    servers: list[dict[str, JsonValue]]
    steps: list[Step]


class Registered(TypedDict):
    client_id: str


REGISTERED: Final[TypeAdapter[Registered]] = TypeAdapter(Registered)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class FrozenDatetime(datetime):
    instant: ClassVar[datetime] = datetime.fromtimestamp(0, UTC)

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        return cls.fromtimestamp(cls.instant.timestamp(), tz)


class ScriptedTokens:
    def __init__(self, values: list[str]) -> None:
        self.values: list[str] = values

    def __call__(self, nbytes: int | None = None) -> str:
        return self.values.pop(0) if self.values and nbytes != 0 else ""


class Answer(Generic[T]):
    def __init__(self, value: T) -> None:
        self.value: T = value

    async def __call__(self, _user_id: str, _server_id: str, /) -> T:
        return self.value


class Teams:
    def __init__(self, step: Step) -> None:
        self.step: Step = step

    async def __call__(self, _user_id: str, /) -> tuple[ConsentTeam, ...] | ReloadFailure:
        if self.step["teams_failure"] is not None:
            return self.step["teams_failure"]
        return tuple(
            ConsentTeam(team_id=row["team_id"], team_alias=row["team_alias"])
            for row in self.step["teams"]
        )


def consent_page(
    *, client_origin: str, user_id: str, teams: Sequence[tuple[str, str]],
    flow_handle: str, complete_url: str,
) -> str:
    page = {
        "client_origin": client_origin, "complete_url": complete_url, "flow_handle": flow_handle,
        "teams": [list(team) for team in teams], "user_id": user_id,
    }
    return json.dumps(page, sort_keys=True, separators=(",", ":"))


def build_request(step: Step, captured: list[str]) -> Request:
    source = step["cookie_from"]
    headers: list[tuple[bytes, bytes]] = [(b"host", b"proxy.test:8443")]
    if source >= 0 and captured[source]:
        headers.append((b"cookie", captured[source].encode()))
    query = step["query"]
    scope: dict[str, str | bytes | list[tuple[bytes, bytes]]] = {
        "type": "http", "method": "GET", "scheme": "https", "path": step["path"],
        "query_string": b"" if query is None else query.encode(), "headers": headers,
    }
    return Request(scope)


def redact(text: str) -> str:
    head, separator, rest = text.partition("code=")
    if not separator:
        return text
    tail = rest.partition("&")[2]
    return head + "code=<code>" + ("" if not tail else "&" + tail)


def cookie_text(header: str | None) -> str | None:
    if header is None:
        return None
    head, _separator, attributes = header.partition("; ")
    name, _equals, _value = head.partition("=")
    fields: dict[str, str] = {"max-age": "0", "path": "", "secure": "false"}
    fields["httponly"] = "false"
    fields["samesite"] = ""
    for attribute in attributes.split("; "):
        key, _equals, text = attribute.lower().partition("=")
        if key in ("secure", "httponly"):
            fields[key] = "true"
        elif key in fields:
            fields[key] = text
    order = ("max-age", "path", "secure", "httponly", "samesite")
    return "|".join([name, *(fields[field] for field in order)])


def response_report(name: str, response: Response) -> tuple[dict[str, JsonValue], str]:
    header = response.headers.get("set-cookie")
    location = response.headers.get("location")
    value = "" if header is None else header.partition("; ")[0]
    pairs: list[list[str]] = []
    for header_name in CACHE_HEADERS:
        found = response.headers.get(header_name)
        if found is not None:
            pairs.append([header_name, found])
    body = None if response.status_code == REDIRECT else redact(bytes(response.body).decode())
    return {
        "name": name,
        "status": response.status_code,
        "location": None if location is None else redact(location),
        "headers": JSON.validate_python(pairs),
        "body": body,
        "cookie": cookie_text(header),
    }, value


def run_step(
    step: Step, clients: list[str], cache: DualCache, captured: list[str]
) -> tuple[dict[str, JsonValue], str]:
    request = build_request(step, captured)
    FrozenDatetime.instant = datetime.fromtimestamp(step["now"], UTC)
    if step["kind"] == "contract":
        contract = native_client_auth_contract(request, step["token_exchange"])
        return {
            "name": step["name"],
            "status": 200,
            "location": None,
            "headers": [],
            "body": json.dumps(contract, separators=(",", ":")),
            "cookie": None,
        }, ""
    secrets.token_urlsafe = ScriptedTokens(step["tokens"])
    client_id = step["client_id"] if step["client"] < 0 else clients[step["client"]]
    if step["kind"] == "aggregate":
        response = aggregate_authorize(
            request,
            client_id,
            step["redirect_uri"],
            step["state"],
            step["code_challenge"],
            step["code_challenge_method"],
            step["response_type"],
            step["user"],
            step["resource"],
        )
    elif step["kind"] == "native":
        response = asyncio.run(
            native_client_authorize(
                request,
                client_id,
                step["redirect_uri"],
                step["state"],
                step["code_challenge"],
                step["code_challenge_method"],
                step["response_type"],
                step["user"],
                Teams(step),
            )
        )
    elif step["kind"] == "describe":
        response = asyncio.run(
            describe_connect_flow(
                request,
                step["handle"],
                step["user"],
                Answer[VendorState](step["vendor"]),
                Answer(step["reachable"]),
            )
        )
    else:
        response = asyncio.run(
            complete_connect_flow(
                request,
                step["handle"],
                step["user"],
                cache,
                step["delivery"],
                step["team_id"],
                step["decision"],
                Answer[VendorState](step["vendor"]),
                Answer(step["reachable"]),
            )
        )
    return response_report(step["name"], response)


def register_clients(seeds: list[ClientSeed]) -> list[str]:
    clients: list[str] = []
    for seed in seeds:
        uris = JSON.validate_python(seed["redirect_uris"])
        body: dict[str, JsonValue] = {"redirect_uris": uris}
        scope: dict[str, str] = {"type": "http", "path": "/register"}
        response = asyncio.run(
            register_aggregate_client(Request(scope), body, token_exchange_available=False)
        )
        clients.append(REGISTERED.validate_json(bytes(response.body))["client_id"])
    return clients


def run(input_value: Input) -> JsonValue:
    os.environ["LITELLM_SALT_KEY"] = "b4675-oracle-salt"
    native_client_consent.render_native_client_consent_page = consent_page
    importlib.reload(gateway_dcr_flow)
    gateway_dcr_flow.datetime = FrozenDatetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/gateway_dcr_flow.py#L709
    clients = register_clients(input_value["clients"])
    servers = [MCPServer.model_validate(raw) for raw in input_value["servers"]]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp_server/mcp_server_manager.py#L27
    global_mcp_server_manager.registry = {server.server_id: server for server in servers}
    cache = DualCache()
    captured: list[str] = []
    reports: list[dict[str, JsonValue]] = []
    for step in input_value["steps"]:
        report, cookie = run_step(step, clients, cache, captured)
        reports.append(report)
        captured.append(cookie)
    return JSON.validate_python(reports)
