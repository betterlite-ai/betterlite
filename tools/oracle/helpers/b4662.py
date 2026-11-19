import asyncio
import base64
import json
import os
from collections.abc import Coroutine
from dataclasses import dataclass
from typing import Final, Literal, NotRequired, Protocol, TypedDict
from urllib.parse import parse_qsl

import httpx
import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy import proxy_server
from litellm.proxy._experimental.mcp_server.oauth2_flow_backfill import (
    BackfillRule,
    backfill_null_oauth2_flows,
    classify_null_flow_row,
)
from litellm.proxy._experimental.mcp_server.oauth2_token_cache import (
    MCPOAuth2TokenCache,
    MCPPerUserTokenCache,
    _compute_per_user_token_ttl,
    resolve_mcp_auth,
    resolved_token_header,
)
from litellm.proxy._experimental.mcp_server.oauth_issuer_stamp_backfill import (
    backfill_discovery_stamped_issuers,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.types.llms.custom_http import httpxSpecialProvider
from litellm.types.mcp import MCPCredentials
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
FIXED_NOW: Final = 1_700_000_000.0


class FixedClock:
    def __call__(self) -> float:
        return FIXED_NOW


class ServerSpec(TypedDict):
    server_id: str
    name: str
    transport: str
    url: NotRequired[str | None]
    auth_type: NotRequired[str | None]
    oauth2_flow: NotRequired[str | None]
    client_id: NotRequired[str | None]
    client_secret: NotRequired[str | None]
    token_url: NotRequired[str | None]
    scopes: NotRequired[list[str] | None]
    upstream_resource: NotRequired[str | None]
    token_endpoint_auth_method: NotRequired[str | None]
    authentication_token: NotRequired[str | None]
    upstream_token_header: NotRequired[str | None]
    token_storage_ttl_seconds: NotRequired[int | None]


class TextOverride(TypedDict):
    kind: Literal["text"]
    value: str


class HeadersOverride(TypedDict):
    kind: Literal["headers"]
    headers: dict[str, str]


AuthOverride = TextOverride | HeadersOverride


class SuccessOutcome(TypedDict):
    kind: Literal["success"]
    body: str


class DeniedOutcome(TypedDict):
    kind: Literal["denied"]
    status_code: int
    detail: str


class UnreachableOutcome(TypedDict):
    kind: Literal["unreachable"]
    detail: str


Outcome = SuccessOutcome | DeniedOutcome | UnreachableOutcome


class SignalSpec(TypedDict):
    has_per_user_tokens: bool
    authorization_url: NotRequired[str | None]
    registration_url: NotRequired[str | None]
    token_url: NotRequired[str | None]
    credentials: NotRequired[dict[str, JsonValue] | None]


class FlowRowSpec(TypedDict):
    server_id: str
    authorization_url: NotRequired[str | None]
    registration_url: NotRequired[str | None]
    token_url: NotRequired[str | None]
    credentials: NotRequired[dict[str, JsonValue] | None]


class CredentialSpec(TypedDict):
    server_id: str
    kind: Literal["oauth2", "key", "garbage"]


class IssuerRowSpec(TypedDict):
    server_id: str
    alias: NotRequired[str | None]
    server_name: NotRequired[str | None]
    auth_type: NotRequired[str | None]
    issuer: NotRequired[str | None]
    authorization_url: NotRequired[str | None]
    token_url: NotRequired[str | None]
    registration_url: NotRequired[str | None]
    updated_by: NotRequired[str | None]


class Call(TypedDict):
    function: Literal[
        "identity",
        "oauth2_get_token",
        "oauth2_fetch",
        "oauth2_invalidate",
        "per_user_get",
        "per_user_set",
        "per_user_delete",
        "per_user_ttl",
        "flow_classify",
        "flow_backfill",
        "issuer_backfill",
        "auth_resolve",
        "token_header",
    ]
    server: NotRequired[ServerSpec]
    servers: NotRequired[list[ServerSpec]]
    outcomes: NotRequired[list[Outcome]]
    calls: NotRequired[int]
    concurrent: NotRequired[bool]
    user_id: NotRequired[str]
    server_id: NotRequired[str]
    seed: NotRequired[str | None]
    garbage: NotRequired[bool]
    fail_get: NotRequired[bool]
    access_token: NotRequired[str]
    ttl: NotRequired[int]
    identity_binding_proof: NotRequired[str | None]
    fail_set: NotRequired[bool]
    fail_delete: NotRequired[bool]
    expires_in: NotRequired[int | None]
    signals: NotRequired[list[SignalSpec]]
    flow_rows: NotRequired[list[FlowRowSpec]]
    issuer_rows: NotRequired[list[IssuerRowSpec]]
    user_credentials: NotRequired[list[CredentialSpec]]
    fail_server_ids: NotRequired[list[str]]
    runs: NotRequired[int]
    mcp_auth_header: NotRequired[AuthOverride | None]


class Captured(TypedDict):
    form: list[list[str]]
    authorization: str | None


class ClientCache(Protocol):
    def set_cache(self, key: str, value: AsyncHTTPHandler) -> None: ...


def form_view(content: bytes) -> list[list[str]]:
    pairs = sorted(parse_qsl(content.decode("utf-8")))
    return [[name, value] for name, value in pairs]


class ScriptedTransport:
    def __init__(self, outcomes: list[Outcome]) -> None:
        self.outcomes = outcomes
        self.calls: list[Captured] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.calls.append(
            {
                "form": form_view(request.content),
                "authorization": request.headers.get("authorization"),
            }
        )
        outcome = self.outcomes.pop(0)
        match outcome:
            case {"kind": "success", "body": body}:
                return httpx.Response(200, json=TypeAdapter(JsonValue).validate_json(body), request=request)
            case {"kind": "denied", "status_code": status, "detail": detail}:
                return httpx.Response(status, text=detail, request=request)
            case {"kind": "unreachable", "detail": detail}:
                raise httpx.ConnectError(detail)


def install_client(transport: ScriptedTransport) -> None:
    holder = LLMClientCache()
    litellm.in_memory_llm_clients_cache = holder
    cache: ClientCache = holder
    cache.set_cache(
        "async_httpx_client" + httpxSpecialProvider.MCP,
        AsyncHTTPHandler(transport=httpx.MockTransport(transport)),
    )


class Memory(InMemoryCache):
    def __init__(self, failure: str) -> None:
        super().__init__(clock=FixedClock())
        self.failure = failure
        self.raw: dict[str, str] = {}
        self.set_ttls: dict[str, int] = {}

    @override
    async def async_get_cache(self, key: str, **kwargs: JsonValue) -> str | None:
        if self.failure == "get":
            raise RuntimeError("the fake cache get failed")
        return self.raw.get(key)

    @override
    async def async_set_cache(self, key: str, value: str, **kwargs: JsonValue) -> None:
        if self.failure == "set":
            raise RuntimeError("the fake cache set failed")
        self.raw[key] = value
        self.set_ttls[key] = TypeAdapter(int).validate_python(kwargs["ttl"])

    @override
    def delete_cache(self, key: str) -> None:
        if self.failure == "delete":
            raise RuntimeError("the fake cache delete failed")
        self.raw.pop(key, None)


def install_user_cache(failure: str) -> Memory:
    memory = Memory(failure)
    os.environ["LITELLM_SALT_KEY"] = "b4662-oracle-salt"
    proxy_server.redis_usage_cache = None
    proxy_server.user_api_key_cache = UserApiKeyCache(in_memory_cache=memory, redis_cache=None)
    return memory


@dataclass
class FlowRow:
    server_id: str
    authorization_url: str | None
    registration_url: str | None
    token_url: str | None
    credentials: dict[str, JsonValue] | None
    oauth2_flow: str | None
    updated_by: str | None
    auth_type: str = "oauth2"


@dataclass
class CredentialRow:
    server_id: str
    credential_b64: str


@dataclass
class IssuerRow:
    server_id: str
    auth_type: str | None
    issuer: str | None
    authorization_url: str | None
    token_url: str | None
    registration_url: str | None
    updated_by: str | None
    alias: str | None = None
    server_name: str | None = None


def declared_ids(where: dict[str, JsonValue]) -> list[str]:
    return TypeAdapter(dict[str, list[str]]).validate_python(where["server_id"])["in"]


def flow_matches(row: FlowRow, where: dict[str, JsonValue]) -> bool:
    if "auth_type" in where and row.auth_type != TypeAdapter(str).validate_python(where["auth_type"]):
        return False
    if "oauth2_flow" in where:
        expected = TypeAdapter(str | None).validate_python(where["oauth2_flow"])
        if row.oauth2_flow != expected:
            return False
    if "server_id" in where and row.server_id not in declared_ids(where):
        return False
    return True


class ServerTable:
    def __init__(self, rows: list[FlowRow]) -> None:
        self.rows = rows
        self.stamps: list[tuple[str, list[str]]] = []

    async def find_many(self, *, where: dict[str, JsonValue]) -> list[FlowRow]:
        return [row for row in self.rows if flow_matches(row, where)]

    async def update_many(self, *, where: dict[str, JsonValue], data: dict[str, JsonValue]) -> None:
        selected = [row for row in self.rows if flow_matches(row, where)]
        flow = TypeAdapter(str).validate_python(data["oauth2_flow"])
        actor = TypeAdapter(str).validate_python(data["updated_by"])
        for row in selected:
            row.oauth2_flow = flow
            row.updated_by = actor
        self.stamps.append((flow, [row.server_id for row in selected]))


class CredentialTable:
    def __init__(self, rows: list[CredentialRow]) -> None:
        self.rows = rows

    async def find_many(self, *, where: dict[str, JsonValue]) -> list[CredentialRow]:
        ids = declared_ids(where)
        return [row for row in self.rows if row.server_id in ids]


def issuer_matches(row: IssuerRow, where: dict[str, JsonValue]) -> bool:
    if "updated_by" in where:
        expected = TypeAdapter(str | None).validate_python(where["updated_by"])
        if row.updated_by != expected:
            return False
    if "auth_type" in where:
        allowed = TypeAdapter(dict[str, list[str]]).validate_python(where["auth_type"])["in"]
        if row.auth_type is None or row.auth_type not in allowed:
            return False
    return True


class IssuerTable:
    def __init__(self, rows: list[IssuerRow], fail: list[str]) -> None:
        self.rows = rows
        self.fail = fail
        self.attempted: list[str] = []
        self.updated: list[str] = []

    async def find_many(self, *, where: dict[str, JsonValue]) -> list[IssuerRow]:
        return [row for row in self.rows if issuer_matches(row, where)]

    async def update(self, *, where: dict[str, JsonValue], data: dict[str, JsonValue]) -> None:
        server_id = TypeAdapter(str).validate_python(where["server_id"])
        self.attempted.append(server_id)
        if server_id in self.fail:
            raise RuntimeError("the fake issuer write failed")
        issuer = TypeAdapter(str | None).validate_python(data["issuer"])
        actor = TypeAdapter(str).validate_python(data["updated_by"])
        for row in self.rows:
            if row.server_id == server_id:
                row.issuer = issuer
                row.updated_by = actor
        self.updated.append(server_id)


class FakeDb(PrismaWrapper):
    def __init__(self, server_table: ServerTable | IssuerTable, credential_table: CredentialTable) -> None:
        self.server_table = server_table
        self.credential_table = credential_table

    @override
    def __getattr__(self, name: str) -> ServerTable | IssuerTable | CredentialTable:
        match name:
            case "litellm_mcpservertable":
                return self.server_table
            case "litellm_mcpusercredentials":
                return self.credential_table
            case _:
                raise AttributeError(name)


class FakeClient(PrismaClient):
    def __init__(self, server_table: ServerTable | IssuerTable, credential_table: CredentialTable) -> None:
        self.db = FakeDb(server_table, credential_table)


def server_of(call: Call) -> MCPServer:
    return TypeAdapter(MCPServer).validate_python(call["server"])


def auth_override(call: Call) -> str | dict[str, str] | None:
    override = call["mcp_auth_header"]
    match override:
        case {"kind": "text", "value": text}:
            return text
        case {"kind": "headers", "headers": headers}:
            return headers
        case None:
            return None


def auth_view(auth: str | dict[str, str] | None) -> JsonValue:
    match auth:
        case str():
            return JSON.validate_python({"kind": "text", "value": auth})
        case dict():
            return JSON.validate_python({"kind": "headers", "headers": auth})
        case None:
            return None


def identity_op(call: Call) -> JsonValue:
    identities = [
        MCPOAuth2TokenCache._token_identity(TypeAdapter(MCPServer).validate_python(spec))  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth2_token_cache.py#L63-L75
        for spec in call["servers"]
    ]
    return JSON.validate_python({"identities": identities})


async def get_token_op(call: Call) -> JsonValue:
    transport = ScriptedTransport(call["outcomes"])
    install_client(transport)
    cache = MCPOAuth2TokenCache()
    server = server_of(call)
    calls = call["calls"]
    tokens: list[str | None] = []
    if call["concurrent"]:
        tasks: list[Coroutine[None, None, str | None]] = [cache.async_get_token(server) for _ in range(calls)]
        tokens.extend(await asyncio.gather(*tasks))
    else:
        for _ in range(calls):
            tokens.append(await cache.async_get_token(server))
    return JSON.validate_python({"tokens": tokens, "posts": len(transport.calls)})


async def fetch_op(call: Call) -> JsonValue:
    transport = ScriptedTransport(call["outcomes"])
    install_client(transport)
    server = server_of(call)
    token, ttl = await MCPOAuth2TokenCache()._fetch_token(server)  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth2_token_cache.py#L117-L190
    captured = transport.calls[0]
    return JSON.validate_python(
        {
            "token": token,
            "ttl": ttl,
            "form": captured["form"],
            "authorization": captured["authorization"],
        }
    )


async def invalidate_op(call: Call) -> JsonValue:
    transport = ScriptedTransport(call["outcomes"])
    install_client(transport)
    cache = MCPOAuth2TokenCache()
    server = server_of(call)
    first = await cache.async_get_token(server)
    cache.invalidate(server.server_id)
    second = await cache.async_get_token(server)
    return JSON.validate_python({"tokens": [first, second], "posts": len(transport.calls)})


async def per_user_get_op(call: Call) -> JsonValue:
    memory = install_user_cache("get" if call["fail_get"] else "none")
    cache = MCPPerUserTokenCache()
    if call["seed"] is not None:
        await cache.set(call["user_id"], call["server_id"], call["seed"], 300)
    if call["garbage"]:
        key = next(iter(memory.raw))
        memory.raw[key] = "garbage"
    token = await cache.get_token(call["user_id"], call["server_id"])
    return JSON.validate_python({"token": None if token is None else token.access_token})


async def per_user_set_op(call: Call) -> JsonValue:
    memory = install_user_cache("set" if call["fail_set"] else "none")
    cache = MCPPerUserTokenCache()
    await cache.set(
        call["user_id"],
        call["server_id"],
        call["access_token"],
        call["ttl"],
        call["identity_binding_proof"],
    )
    token = await cache.get_token(call["user_id"], call["server_id"])
    stored = next(iter(memory.raw.values()), None)
    ttl = next(iter(memory.set_ttls.values()), None)
    return JSON.validate_python(
        {
            "token": None if token is None else token.access_token,
            "raw_differs": stored is not None and stored != call["access_token"],
            "ttl": ttl,
        }
    )


async def per_user_delete_op(call: Call) -> JsonValue:
    install_user_cache("delete" if call["fail_delete"] else "none")
    cache = MCPPerUserTokenCache()
    await cache.set(call["user_id"], call["server_id"], "token-value", 300)
    await cache.delete(call["user_id"], call["server_id"])
    token = await cache.get_token(call["user_id"], call["server_id"])
    return JSON.validate_python({"token_after": None if token is None else token.access_token})


def per_user_ttl_op(call: Call) -> JsonValue:
    server = server_of(call)
    return JSON.validate_python({"ttl": _compute_per_user_token_ttl(server, call["expires_in"])})


def credentials_of(raw: dict[str, JsonValue] | None) -> MCPCredentials | None:
    if raw is None:
        return None
    return TypeAdapter(MCPCredentials).validate_python(raw)


def classify_op(call: Call) -> JsonValue:
    results: list[JsonValue] = []
    for signal in call["signals"]:
        flow, rule = classify_null_flow_row(
            has_per_user_tokens=signal["has_per_user_tokens"],
            authorization_url=signal["authorization_url"],
            registration_url=signal["registration_url"],
            token_url=signal["token_url"],
            credentials=credentials_of(signal["credentials"]),
        )
        results.append(JSON.validate_python({"flow": flow, "rule": rule}))
    return JSON.validate_python({"results": results})


def flow_row_of(spec: FlowRowSpec) -> FlowRow:
    return FlowRow(
        server_id=spec["server_id"],
        authorization_url=spec["authorization_url"],
        registration_url=spec["registration_url"],
        token_url=spec["token_url"],
        credentials=spec["credentials"],
        oauth2_flow=None,
        updated_by=None,
    )


def credential_row_of(spec: CredentialSpec) -> CredentialRow:
    match spec["kind"]:
        case "oauth2":
            payload = json.dumps({"type": "oauth2", "access_token": "tok"})
        case "key":
            payload = "a-plain-api-key"
        case "garbage":
            payload = "!!!not-base64!!!"
    return CredentialRow(server_id=spec["server_id"], credential_b64=base64.b64encode(payload.encode()).decode())


def flow_row_view(row: FlowRow) -> JsonValue:
    return JSON.validate_python(
        {"server_id": row.server_id, "oauth2_flow": row.oauth2_flow, "updated_by": row.updated_by}
    )


def flow_row_order(row: FlowRow) -> str:
    return row.server_id


async def flow_backfill_op(call: Call) -> JsonValue:
    table = ServerTable([flow_row_of(spec) for spec in call["flow_rows"]])
    credentials = CredentialTable([credential_row_of(spec) for spec in call["user_credentials"]])
    counts: dict[BackfillRule, int] = await backfill_null_oauth2_flows(FakeClient(table, credentials))
    stamps = [{"flow": flow, "server_ids": sorted(ids)} for flow, ids in table.stamps]
    return JSON.validate_python(
        {
            "counts": dict(counts),
            "rows": [flow_row_view(row) for row in sorted(table.rows, key=flow_row_order)],
            "stamps": stamps,
            "ambiguous": sorted([row.server_id for row in table.rows if row.oauth2_flow is None]),
        }
    )


def issuer_row_of(spec: IssuerRowSpec) -> IssuerRow:
    return IssuerRow(
        server_id=spec["server_id"],
        auth_type=spec["auth_type"],
        issuer=spec["issuer"],
        authorization_url=spec["authorization_url"],
        token_url=spec["token_url"],
        registration_url=spec["registration_url"],
        updated_by=spec["updated_by"],
        alias=spec["alias"],
        server_name=spec["server_name"],
    )


def issuer_row_view(row: IssuerRow) -> JsonValue:
    return JSON.validate_python(
        {"server_id": row.server_id, "issuer": row.issuer, "updated_by": row.updated_by}
    )


def issuer_row_order(row: IssuerRow) -> str:
    return row.server_id


async def issuer_backfill_op(call: Call) -> JsonValue:
    table = IssuerTable([issuer_row_of(spec) for spec in call["issuer_rows"]], call["fail_server_ids"])
    healed = 0
    for _run in range(call["runs"]):
        healed += await backfill_discovery_stamped_issuers(FakeClient(table, CredentialTable([])))
    return JSON.validate_python(
        {
            "healed": healed,
            "rows": [issuer_row_view(row) for row in sorted(table.rows, key=issuer_row_order)],
            "updated": sorted(table.updated),
            "failed": sorted([server_id for server_id in table.attempted if server_id not in table.updated]),
        }
    )


async def resolve_op(call: Call) -> JsonValue:
    transport = ScriptedTransport(call["outcomes"])
    install_client(transport)
    server = server_of(call)
    auth = await resolve_mcp_auth(server, auth_override(call))
    return JSON.validate_python({"auth": auth_view(auth), "posts": len(transport.calls)})


def header_op(call: Call) -> JsonValue:
    server = server_of(call)
    return JSON.validate_python({"header": resolved_token_header(server, auth_override(call))})


async def dispatch(call: Call) -> JsonValue:
    match call["function"]:
        case "identity":
            return identity_op(call)
        case "oauth2_get_token":
            return await get_token_op(call)
        case "oauth2_fetch":
            return await fetch_op(call)
        case "oauth2_invalidate":
            return await invalidate_op(call)
        case "per_user_get":
            return await per_user_get_op(call)
        case "per_user_set":
            return await per_user_set_op(call)
        case "per_user_delete":
            return await per_user_delete_op(call)
        case "per_user_ttl":
            return per_user_ttl_op(call)
        case "flow_classify":
            return classify_op(call)
        case "flow_backfill":
            return await flow_backfill_op(call)
        case "issuer_backfill":
            return await issuer_backfill_op(call)
        case "auth_resolve":
            return await resolve_op(call)
        case "token_header":
            return header_op(call)


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    return asyncio.run(dispatch(TypeAdapter(Call).validate_python(input_value)))
