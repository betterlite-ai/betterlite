import asyncio
import importlib
import json
import os
import secrets
from collections.abc import Sequence
from typing import Final, Literal, TypedDict

from litellm.caching.dual_cache import DualCache
from litellm.proxy._experimental.mcp_server import gateway_dcr_flow, native_client_consent
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
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.responses import Response

SALT: Final = "b4675-oracle-salt"
REDIRECT: Final = 303
CACHE_HEADERS: Final = ("cache-control", "content-security-policy", "pragma", "x-frame-options")

Kind = Literal["aggregate", "native", "describe", "complete", "contract"]
VendorState = Literal["present", "absent", "unavailable"]
ReloadFailure = Literal["unresolvable", "unavailable", "faulted", "no_active_key"]


class ClientSeed(TypedDict):
    redirect_uris: list[str]


class TeamSeed(TypedDict):
    team_id: str
    team_alias: str | None


class Step(TypedDict, total=False):
    name: str
    kind: Kind
    path: str
    query: str | None
    cookies: list[list[str]]
    client: int
    client_id: str
    redirect_uri: str
    state: str
    code_challenge: str
    code_challenge_method: str
    response_type: str
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


class FlowFields(TypedDict, total=False):
    user_id: str
    client_id: str
    redirect_uri: str
    state: str
    code_challenge: str
    jti: str
    exp: int
    resource_server_id: str | None
    audience: str | None


class RegisteredBody(TypedDict):
    client_id: str


REGISTERED: Final[TypeAdapter[RegisteredBody]] = TypeAdapter(RegisteredBody)
FLOW: Final[TypeAdapter[FlowFields]] = TypeAdapter(FlowFields)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ScriptedTokens:
    def __init__(self, values: list[str]) -> None:
        self.values = values

    def __call__(self, nbytes: int | None = None) -> str:
        return self.values.pop(0) if self.values and nbytes != 0 else ""


class Vendor:
    def __init__(self, state: VendorState) -> None:
        self.state = state

    async def __call__(self, _user_id: str, _server_id: str, /) -> VendorState:
        return self.state


class Reachable:
    def __init__(self, reachable: bool) -> None:
        self.reachable = reachable

    async def __call__(self, _user_id: str, _server_id: str, /) -> bool:
        return self.reachable


class Teams:
    def __init__(self, rows: list[TeamSeed], failure: ReloadFailure | None) -> None:
        self.rows = rows
        self.failure = failure

    async def __call__(self, _user_id: str, /) -> tuple[ConsentTeam, ...] | ReloadFailure:
        if self.failure is not None:
            return self.failure
        return tuple(ConsentTeam(team_id=row["team_id"], team_alias=row["team_alias"]) for row in self.rows)


def consent_page(
    *, client_origin: str, user_id: str, teams: Sequence[tuple[str, str]], flow_handle: str, complete_url: str
) -> str:
    page = {
        "client_origin": client_origin,
        "complete_url": complete_url,
        "flow_handle": flow_handle,
        "teams": [list(team) for team in teams],
        "user_id": user_id,
    }
    return json.dumps(page, sort_keys=True, separators=(",", ":"))


def build_request(step: Step) -> Request:
    cookie = "; ".join(f"{pair[0]}={pair[1]}" for pair in step.get("cookies", []))
    headers: list[tuple[bytes, bytes]] = [] if not cookie else [(b"cookie", cookie.encode())]
    query = step.get("query") or ""
    scope: dict[str, object] = {
        "type": "http",
        "scheme": "https",
        "server": ("proxy.test", 8443),
        "path": step.get("path", "/authorize"),
        "root_path": "",
        "query_string": query.encode(),
        "headers": headers,
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
    fields: dict[str, str] = {"max-age": "0", "path": "", "secure": "false", "httponly": "false", "samesite": ""}
    for attribute in attributes.split("; "):
        key, _equals, text = attribute.lower().partition("=")
        if key in ("secure", "httponly"):
            fields[key] = "true"
        elif key in fields:
            fields[key] = text
    return "|".join([name, fields["max-age"], fields["path"], fields["secure"], fields["httponly"], fields["samesite"]])


def flow_fields(value: str | None) -> dict[str, JsonValue] | None:
    if not value:
        return None
    opened = gateway_dcr_flow._open_sealed(value, "", gateway_dcr_flow._ConnectFlow, gateway_dcr_flow._CONNECT_FLOW_DEBUG_KEY)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/gateway_dcr_flow.py#L355
    if opened is None:
        return None
    fields = FLOW.validate_json(opened.model_dump_json())
    return {
        "user_id": fields["user_id"],
        "client_id": fields["client_id"],
        "redirect_uri": fields["redirect_uri"],
        "state": fields["state"],
        "code_challenge": fields["code_challenge"],
        "jti": fields["jti"],
        "exp": fields["exp"],
        "resource_server_id": fields.get("resource_server_id"),
        "audience": fields.get("audience"),
    }


def response_report(name: str, response: Response) -> dict[str, JsonValue]:
    header = response.headers.get("set-cookie")
    location = response.headers.get("location")
    value = None if header is None else header.partition("; ")[0].partition("=")[2]
    pairs = [
        [header_name, header_value]
        for header_name, header_value in response.headers.items()
        if header_name in CACHE_HEADERS
    ]
    return {
        "name": name,
        "status": response.status_code,
        "location": None if location is None else redact(location),
        "headers": JSON.validate_python(sorted(pairs)),
        "body": None if response.status_code == REDIRECT else redact(bytes(response.body).decode()),
        "cookie": cookie_text(header),
        "flow": flow_fields(value),
    }


def run_step(step: Step, clients: list[str], cache: DualCache) -> dict[str, JsonValue]:
    request = build_request(step)
    if step["kind"] == "contract":
        contract = native_client_auth_contract(request, step["token_exchange"])
        return {
            "name": step["name"],
            "status": 200,
            "location": None,
            "headers": JSON.validate_python([]),
            "body": json.dumps(contract, separators=(",", ":")),
            "cookie": None,
            "flow": None,
        }
    secrets.token_urlsafe = ScriptedTokens(step["tokens"])
    client_id = step["client_id"] if "client" not in step else clients[step["client"]]
    redirect_uri = step["redirect_uri"]
    supplier = Vendor(step.get("vendor", "unavailable"))
    reachability = Reachable(step.get("reachable", True))
    if step["kind"] == "aggregate":
        response = aggregate_authorize(
            request,
            client_id,
            redirect_uri,
            step["state"],
            step.get("code_challenge"),
            step.get("code_challenge_method"),
            step.get("response_type"),
            step.get("user"),
            step.get("resource"),
        )
    elif step["kind"] == "native":
        response = asyncio.run(
            native_client_authorize(
                request,
                client_id,
                redirect_uri,
                step["state"],
                step.get("code_challenge"),
                step.get("code_challenge_method"),
                step.get("response_type"),
                step.get("user"),
                Teams(step.get("teams", []), step.get("teams_failure")),
            )
        )
    elif step["kind"] == "describe":
        response = asyncio.run(
            describe_connect_flow(request, step["handle"], step.get("user"), supplier, reachability)
        )
    else:
        response = asyncio.run(
            complete_connect_flow(
                request,
                step["handle"],
                step.get("user"),
                cache,
                step.get("delivery"),
                step.get("team_id"),
                step.get("decision"),
                supplier,
                reachability,
            )
        )
    return response_report(step["name"], response)


def register_clients(seeds: list[ClientSeed]) -> list[str]:
    clients: list[str] = []
    for seed in seeds:
        request = Request({"type": "http", "path": "/register", "headers": []})
        response = asyncio.run(register_aggregate_client(request, {"redirect_uris": seed["redirect_uris"]}, False))
        clients.append(REGISTERED.validate_json(response.body)["client_id"])
    return clients


def run(input_value: Input) -> JsonValue:
    os.environ["LITELLM_SALT_KEY"] = SALT
    native_client_consent.render_native_client_consent_page = consent_page
    importlib.reload(gateway_dcr_flow)
    clients = register_clients(input_value["clients"])
    servers = [MCPServer.model_validate(raw) for raw in input_value["servers"]]
    global_mcp_server_manager.registry = {server.server_id: server for server in servers}
    cache = DualCache()
    reports = [run_step(step, clients, cache) for step in input_value["steps"]]
    return JSON.validate_python(reports)
