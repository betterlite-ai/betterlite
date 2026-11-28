import asyncio
import importlib
import os
from datetime import UTC, datetime, tzinfo
from typing import ClassVar, Final, Literal, NamedTuple, Self, TypedDict
from urllib.parse import urlparse

from litellm.caching.dual_cache import DualCache
from litellm.proxy import proxy_server
from litellm.proxy._experimental.mcp_server import gateway_dcr_flow
from litellm.proxy._experimental.mcp_server.gateway_dcr_flow import register_aggregate_client
from litellm.proxy._experimental.mcp_server.mcp_server_manager import global_mcp_server_manager
from litellm.proxy._experimental.mcp_server.outbound_credentials import session_credentials
from litellm.proxy._experimental.mcp_server.outbound_credentials import session_token
from litellm.proxy._experimental.mcp_server.outbound_credentials.session_token import (
    NotASessionToken,
    SessionBadSignature,
    SessionExpired,
    SessionMalformed,
)
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from typing_extensions import override

MASTER: Final = "sk-b4676-oracle-master-key"
BODY: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ClientSeed(TypedDict):
    redirect_uris: list[str]


class MintAnswer(TypedDict):
    key: str
    expires_in: int
    user_id: str
    team_id: str | None


class ExchangeAnswer(TypedDict):
    user_id: str
    team_id: str | None


class ExchangeFailure(TypedDict):
    error: Literal["unsupported_grant_type", "invalid_request", "temporarily_unavailable"]
    description: str


class SetupStep(TypedDict):
    kind: Literal["mint_code", "mint_token"]
    user_id: str
    client: int
    redirect_uri: str
    code_challenge: str
    jti: str
    iat: int
    exp: int
    resource_server_id: str | None
    audience: session_token.SessionAudience | None
    team_id: str | None
    at: int
    token_kind: session_token.SessionTokenKind
    tamper: bool
    bearer: bool


class Step(TypedDict):
    name: str
    kind: Literal["token", "revoke", "introspect", "pkce"]
    master: Literal["present", "absent", "invalid"]
    now: int
    client: int
    grant_type: str
    code: int
    redirect_uri: str | None
    code_verifier: str
    code_challenge: str
    refresh: int
    resource: str | None
    subject: int
    subject_token_type: str | None
    requested_token_type: str | None
    token: int
    reload: gateway_dcr_flow.ReloadUserFailure | None
    mint: MintAnswer | None
    mint_failure: gateway_dcr_flow.ProxyCredentialMintFailure | None
    exchange: ExchangeAnswer | None
    exchange_failure: ExchangeFailure | None


class Input(TypedDict):
    base_url: str
    clients: list[ClientSeed]
    servers: list[dict[str, JsonValue]]
    setup: list[SetupStep]
    steps: list[Step]


class Context(NamedTuple):
    base_url: str
    clients: list[str]
    keys: session_token.SessionSigningKeys
    cache: DualCache
    products: list[str]


class Clock(datetime):
    at: ClassVar[int] = 0

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        return cls.fromtimestamp(cls.at, tz)


class Answers(NamedTuple):
    step: Step

    async def reload(self, _user_id: str, /) -> gateway_dcr_flow.ReloadUserFailure | None:
        return self.step["reload"]

    async def mint(
        self, _user_id: str, _team_id: str | None, /
    ) -> gateway_dcr_flow.MintedProxyCredential | gateway_dcr_flow.ProxyCredentialMintFailure:
        if self.step["mint_failure"] is not None:
            return self.step["mint_failure"]
        answer = self.step["mint"]
        if answer is None:
            raise ValueError("the case lists no mint answer")
        return gateway_dcr_flow.MintedProxyCredential(**answer)

    async def exchange(
        self, _subject_token: str, _request: Request, /
    ) -> gateway_dcr_flow.SubjectIdentity | gateway_dcr_flow.SubjectTokenRefusal:
        if self.step["exchange_failure"] is not None:
            return gateway_dcr_flow.SubjectTokenRefusal(**self.step["exchange_failure"])
        answer = self.step["exchange"]
        if answer is None:
            raise ValueError("the case lists no exchange answer")
        return gateway_dcr_flow.SubjectIdentity(**answer)


def entry(entries: list[str], index: int) -> str:
    return entries[index] if 0 <= index < len(entries) else ""


def register_clients(seeds: list[ClientSeed]) -> list[str]:
    clients: list[str] = []
    for seed in seeds:
        body: dict[str, object] = {"redirect_uris": seed["redirect_uris"]}
        scope: dict[str, str] = {"type": "http", "path": "/register"}
        response = asyncio.run(
            register_aggregate_client(Request(scope), body, token_exchange_available=False)
        )
        clients.append(TEXT.validate_python(BODY.validate_json(bytes(response.body))["client_id"]))
    return clients


def claims(token: str, kind: str, context: Context) -> JsonValue:
    at = datetime.fromtimestamp(Clock.at, UTC)
    match kind:
        case "session":
            opened = session_token.open_session_token(token, context.keys, at)
        case _:
            opened = session_token.open_session_refresh_token(token, context.keys, at)
    match opened:
        case session_token.OpenedSessionToken():
            principal = opened.principal
            return {
                "user_id": principal.user_id,
                "client_id": {"sealed_client": context.clients.index(principal.client_id)},
                "resource_server_id": principal.resource_server_id,
                "audience": principal.audience, "kind": opened.kind,
                "iat": opened.iat, "exp": opened.exp,
            }
        case NotASessionToken() | SessionBadSignature() | SessionExpired() | SessionMalformed():
            return "<unopenable>"


def body_value(step: Step, body: dict[str, JsonValue], context: Context) -> JsonValue:
    kind = step["kind"]
    if "error" in body or kind == "revoke":
        return body
    if kind == "introspect":
        value = dict(body)
        if "jti" in value:
            value["jti"] = "<jti>"
        if "client_id" in value:
            sealed = TEXT.validate_python(value["client_id"])
            value["client_id"] = {"sealed_client": context.clients.index(sealed)}
        return value
    refresh = claims(TEXT.validate_python(body["refresh_token"]), "session_refresh", context)
    value: dict[str, JsonValue] = {
        "refresh_token": refresh, "token_type": body["token_type"],
        "expires_in": body["expires_in"],
    }
    if "user_id" in body:
        value["access_token"] = body["access_token"]
        value["user_id"] = body["user_id"]
        value["team_id"] = body["team_id"]
        if "issued_token_type" in body:
            value["issued_token_type"] = body["issued_token_type"]
    else:
        access = claims(TEXT.validate_python(body["access_token"]), "session", context)
        value["access_token"] = access
    return value


def run_setup(step: SetupStep, context: Context) -> str:
    client = entry(context.clients, step["client"])
    if step["kind"] == "mint_code":
        code = gateway_dcr_flow._GatewayAuthCode(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/gateway_dcr_flow.py#L307
            user_id=step["user_id"], client_id=client,
            redirect_uri=step["redirect_uri"], code_challenge=step["code_challenge"],
            jti=step["jti"], iat=step["iat"], audience=step["audience"],
            exp=step["exp"], resource_server_id=step["resource_server_id"],
            team_id=step["team_id"],
        )
        return gateway_dcr_flow._seal(gateway_dcr_flow.GATEWAY_AUTH_CODE_PREFIX, code)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/gateway_dcr_flow.py#L342
    principal = session_token.SessionPrincipal(
        user_id=step["user_id"], client_id=client,
        resource_server_id=step["resource_server_id"],
        audience=step["audience"], team_id=step["team_id"],
    )
    at = datetime.fromtimestamp(step["at"], UTC)
    match step["token_kind"]:
        case "session":
            minted = session_token.mint_session_token(principal, context.keys, at)
        case _:
            minted = session_token.mint_session_refresh_token(principal, context.keys, at)
    match minted:
        case session_token.MintedSessionToken():
            token = minted.token.get_secret_value()
        case session_token.SessionTokenTooLarge():
            raise ValueError("the setup token is too large")
    return ("Bearer " if step["bearer"] else "") + token + ("x" if step["tamper"] else "")


async def run_step(step: Step, context: Context) -> dict[str, JsonValue]:
    Clock.at = step["now"]
    if step["kind"] == "pkce":
        matched = gateway_dcr_flow._pkce_verifier_matches(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/gateway_dcr_flow.py#L965
            step["code_verifier"], step["code_challenge"],
        )
        return {"name": step["name"], "status": 200, "headers": [], "body": {"matches": matched}}
    signing = {"mcp_session_token_signing": {"algorithm": "HS256"}}
    proxy_server.general_settings = signing if step["master"] == "invalid" else {}
    master = None if step["master"] == "absent" else MASTER
    answers = Answers(step)
    token = entry(context.products, step["token"])
    client = entry(context.clients, step["client"])
    if step["kind"] == "token":
        parsed = urlparse(context.base_url)
        scope: dict[str, str | bytes | list[tuple[bytes, bytes]]] = {
            "type": "http", "method": "POST", "scheme": parsed.scheme, "path": "/token",
            "query_string": b"", "headers": [(b"host", parsed.netloc.encode())],
        }
        response = await gateway_dcr_flow.aggregate_token(
            Request(scope), step["grant_type"],
            entry(context.products, step["code"]), step["redirect_uri"],
            client, step["code_verifier"],
            entry(context.products, step["refresh"]), master,
            answers.reload, context.cache, step["resource"], answers.mint,
            entry(context.products, step["subject"]),
            step["subject_token_type"], step["requested_token_type"], answers.exchange,
        )
    elif step["kind"] == "revoke":
        response = await gateway_dcr_flow.revoke_refresh_token(token, client, master, context.cache)
    else:
        response = await gateway_dcr_flow.introspect_gateway_token(
            token, master, answers.reload, context.cache,
        )
    names = ("cache-control", "pragma")
    pairs = [[name, response.headers[name]] for name in names if name in response.headers]
    return {
        "name": step["name"], "status": response.status_code,
        "headers": JSON.validate_python(pairs),
        "body": body_value(step, BODY.validate_json(bytes(response.body)), context),
    }


def run(input_value: Input) -> JsonValue:
    os.environ["LITELLM_SALT_KEY"] = "b4676-oracle-salt"
    importlib.reload(gateway_dcr_flow)
    gateway_dcr_flow.datetime = Clock  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/gateway_dcr_flow.py#L1209
    proxy_server.redis_usage_cache = None
    proxy_server.general_settings = {}
    clients = register_clients(input_value["clients"])
    keys = session_credentials.session_keys_from_master_key(MASTER)
    context = Context(input_value["base_url"], clients, keys, DualCache(), [])
    servers = [MCPServer.model_validate(raw) for raw in input_value["servers"]]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/mcp_server/mcp_server_manager.py#L27
    global_mcp_server_manager.registry = {server.server_id: server for server in servers}
    context.products.extend(run_setup(setup, context) for setup in input_value["setup"])
    reports = [asyncio.run(run_step(step, context)) for step in input_value["steps"]]
    return JSON.validate_python(reports)
