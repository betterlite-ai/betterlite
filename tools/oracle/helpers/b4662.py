import asyncio
import os
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from urllib.parse import parse_qsl

import httpx
import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy import proxy_server
from litellm.proxy._experimental.mcp_server.oauth2_flow_backfill import (
    backfill_null_oauth2_flows,
    classify_null_flow_row,
)
from litellm.proxy._experimental.mcp_server.oauth2_token_cache import (
    MCPOAuth2TokenCache,
    MCPPerUserTokenCache,
    _compute_per_user_token_ttl,  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth2_token_cache.py#L206-L221
    resolve_mcp_auth,
    resolved_token_header,
)
from litellm.proxy._experimental.mcp_server.oauth_issuer_stamp_backfill import (
    backfill_discovery_stamped_issuers,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.types.mcp import MCPCredentials
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON = TypeAdapter(JsonValue)
STR = TypeAdapter(str)
OSTR = TypeAdapter(str | None)
INT = TypeAdapter(int)
BOOL = TypeAdapter(bool)
SERVER = TypeAdapter(MCPServer)
DICT = TypeAdapter(dict[str, JsonValue])
DICTS = TypeAdapter(list[dict[str, JsonValue]])
IDS = TypeAdapter(dict[str, list[str]])
OVERRIDE = TypeAdapter(str | dict[str, str] | None)
AMOUNT = TypeAdapter(int | None)


class ScriptedClient(LLMClientCache):
    def __init__(self, outcomes: list[dict[str, JsonValue]]) -> None:
        super().__init__()
        self.outcomes = outcomes
        self.posts = 0
        self.form: JsonValue = None
        self.authorization: str | None = None
        self.handler = AsyncHTTPHandler(transport=httpx.MockTransport(self.respond))
        litellm.in_memory_llm_clients_cache = self

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> AsyncHTTPHandler:
        return self.handler

    def respond(self, request: httpx.Request) -> httpx.Response:
        pairs = sorted(parse_qsl(request.content.decode("utf-8")))
        self.posts += 1
        self.form = JSON.validate_python([[name, value] for name, value in pairs])
        self.authorization = request.headers.get("authorization")
        outcome = self.outcomes.pop(0)
        kind = STR.validate_python(outcome["kind"])
        if kind == "success":
            return httpx.Response(200, json=JSON.validate_json(STR.validate_python(outcome["body"])), request=request)
        if kind == "denied":
            status = INT.validate_python(outcome["status_code"])
            return httpx.Response(status, text=STR.validate_python(outcome["detail"]), request=request)
        raise httpx.ConnectError(STR.validate_python(outcome["detail"]))


class Memory(InMemoryCache):
    def __init__(self, failure: str) -> None:
        super().__init__()
        self.failure = failure
        self.raw: dict[str, str] = {}
        self.ttls: dict[str, int] = {}

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
        self.ttls[key] = INT.validate_python(kwargs["ttl"])

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


@dataclass(order=True)
class Row:
    server_id: str
    alias: str | None = None
    server_name: str | None = None
    auth_type: str | None = None
    issuer: str | None = None
    authorization_url: str | None = None
    registration_url: str | None = None
    token_url: str | None = None
    updated_by: str | None = None
    credentials: dict[str, JsonValue] | None = None
    oauth2_flow: str | None = None
    credential_b64: str | None = None


class Table:
    def __init__(self, rows: list[Row]) -> None:
        self.rows = rows


class ServerTable(Table):
    def __init__(self, rows: list[Row]) -> None:
        super().__init__(rows)
        self.stamps: list[tuple[str, list[str]]] = []

    async def find_many(self, **_kwargs: JsonValue) -> list[Row]:
        return [row for row in self.rows if row.oauth2_flow is None]

    async def update_many(self, *, where: dict[str, JsonValue], data: dict[str, JsonValue]) -> None:
        ids = IDS.validate_python(where["server_id"])["in"]
        selected = [row for row in self.rows if row.server_id in ids and row.oauth2_flow is None]
        flow = STR.validate_python(data["oauth2_flow"])
        self.stamps.append((flow, [row.server_id for row in selected]))
        for row in selected:
            row.oauth2_flow = flow
            row.updated_by = STR.validate_python(data["updated_by"])


class CredentialTable(Table):
    async def find_many(self, *, where: dict[str, JsonValue]) -> list[Row]:
        ids = IDS.validate_python(where["server_id"])["in"]
        return [row for row in self.rows if row.server_id in ids]


class IssuerTable(Table):
    def __init__(self, rows: list[Row], fail: list[str]) -> None:
        super().__init__(rows)
        self.fail = fail
        self.attempted: list[str] = []
        self.updated: list[str] = []

    async def find_many(self, **_kwargs: JsonValue) -> list[Row]:
        return [row for row in self.rows if row.updated_by == "mcp_oauth_discovery"]

    async def update(self, *, where: dict[str, JsonValue], data: dict[str, JsonValue]) -> None:
        server_id = STR.validate_python(where["server_id"])
        self.attempted.append(server_id)
        if server_id in self.fail:
            raise RuntimeError("the fake issuer write failed")
        for row in self.rows:
            if row.server_id == server_id:
                row.issuer = OSTR.validate_python(data["issuer"])
                row.updated_by = STR.validate_python(data["updated_by"])
        self.updated.append(server_id)


class FakeDb(PrismaWrapper):
    def __init__(self, servers: Table, credentials: Table) -> None:
        self.litellm_mcpservertable = servers
        self.litellm_mcpusercredentials = credentials


class FakeClient(PrismaClient):
    def __init__(self, servers: Table, credentials: Table) -> None:
        self.db = FakeDb(servers, credentials)


def row_of(spec: dict[str, JsonValue]) -> Row:
    credentials = spec.get("credentials")
    return Row(
        server_id=STR.validate_python(spec["server_id"]),
        alias=OSTR.validate_python(spec.get("alias")),
        server_name=OSTR.validate_python(spec.get("server_name")),
        auth_type=OSTR.validate_python(spec.get("auth_type")),
        issuer=OSTR.validate_python(spec.get("issuer")),
        authorization_url=OSTR.validate_python(spec.get("authorization_url")),
        registration_url=OSTR.validate_python(spec.get("registration_url")),
        token_url=OSTR.validate_python(spec.get("token_url")),
        updated_by=OSTR.validate_python(spec.get("updated_by")),
        credentials=None if credentials is None else DICT.validate_python(credentials),
        credential_b64=OSTR.validate_python(spec.get("credential_b64")),
    )


def per_user(call: dict[str, JsonValue]) -> tuple[MCPPerUserTokenCache, str, str]:
    cache = MCPPerUserTokenCache()
    return cache, STR.validate_python(call["user_id"]), STR.validate_python(call["server_id"])


async def identity_op(call: dict[str, JsonValue]) -> JsonValue:
    identities = [
        MCPOAuth2TokenCache._token_identity(server)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth2_token_cache.py#L66-L81
        for server in TypeAdapter(list[MCPServer]).validate_python(call["servers"])
    ]
    return JSON.validate_python({"identities": identities})


async def get_token_op(call: dict[str, JsonValue]) -> JsonValue:
    client = ScriptedClient(DICTS.validate_python(call["outcomes"]))
    cache = MCPOAuth2TokenCache()
    server = SERVER.validate_python(call["server"])
    calls = INT.validate_python(call["calls"])
    if BOOL.validate_python(call["concurrent"]):
        tokens = list(await asyncio.gather(*[cache.async_get_token(server) for _ in range(calls)]))
    else:
        tokens = [await cache.async_get_token(server) for _ in range(calls)]
    return JSON.validate_python({"tokens": tokens, "posts": client.posts})


async def fetch_op(call: dict[str, JsonValue]) -> JsonValue:
    client = ScriptedClient(DICTS.validate_python(call["outcomes"]))
    server = SERVER.validate_python(call["server"])
    token, ttl = await MCPOAuth2TokenCache()._fetch_token(server)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth2_token_cache.py#L117-L190
    return JSON.validate_python(
        {"token": token, "ttl": ttl, "form": client.form, "authorization": client.authorization}
    )


async def invalidate_op(call: dict[str, JsonValue]) -> JsonValue:
    client = ScriptedClient(DICTS.validate_python(call["outcomes"]))
    cache = MCPOAuth2TokenCache()
    server = SERVER.validate_python(call["server"])
    first = await cache.async_get_token(server)
    cache.invalidate(server.server_id)
    second = await cache.async_get_token(server)
    return JSON.validate_python({"tokens": [first, second], "posts": client.posts})


async def per_user_get_op(call: dict[str, JsonValue]) -> JsonValue:
    memory = install_user_cache("get" if BOOL.validate_python(call["fail_get"]) else "none")
    cache, user_id, server_id = per_user(call)
    seed = OSTR.validate_python(call["seed"])
    if seed is not None:
        await cache.set(user_id, server_id, seed, 300)
    if BOOL.validate_python(call["garbage"]):
        memory.raw[next(iter(memory.raw))] = "garbage"
    token = await cache.get_token(user_id, server_id)
    return JSON.validate_python({"token": None if token is None else token.access_token})


async def per_user_set_op(call: dict[str, JsonValue]) -> JsonValue:
    memory = install_user_cache("set" if BOOL.validate_python(call["fail_set"]) else "none")
    cache, user_id, server_id = per_user(call)
    access_token = STR.validate_python(call["access_token"])
    await cache.set(
        user_id,
        server_id,
        access_token,
        INT.validate_python(call["ttl"]),
        OSTR.validate_python(call["identity_binding_proof"]),
    )
    stored = next(iter(memory.raw.values()), None)
    token = await cache.get_token(user_id, server_id)
    return JSON.validate_python(
        {
            "token": None if token is None else token.access_token,
            "raw_differs": stored is not None and stored != access_token,
            "ttl": next(iter(memory.ttls.values()), None),
        }
    )


async def per_user_delete_op(call: dict[str, JsonValue]) -> JsonValue:
    install_user_cache("delete" if BOOL.validate_python(call["fail_delete"]) else "none")
    cache, user_id, server_id = per_user(call)
    await cache.set(user_id, server_id, "token-value", 300)
    await cache.delete(user_id, server_id)
    token = await cache.get_token(user_id, server_id)
    return JSON.validate_python({"token_after": None if token is None else token.access_token})


async def per_user_ttl_op(call: dict[str, JsonValue]) -> JsonValue:
    server = SERVER.validate_python(call["server"])
    ttl = _compute_per_user_token_ttl(server, AMOUNT.validate_python(call["expires_in"]))
    return JSON.validate_python({"ttl": ttl})


def classify_op(call: dict[str, JsonValue]) -> JsonValue:
    results: list[JsonValue] = []
    for signal in DICTS.validate_python(call["signals"]):
        raw = signal.get("credentials")
        credentials = None if raw is None else TypeAdapter(MCPCredentials).validate_python(raw)
        flow, rule = classify_null_flow_row(
            has_per_user_tokens=BOOL.validate_python(signal["has_per_user_tokens"]),
            authorization_url=OSTR.validate_python(signal.get("authorization_url")),
            registration_url=OSTR.validate_python(signal.get("registration_url")),
            token_url=OSTR.validate_python(signal.get("token_url")),
            credentials=credentials,
        )
        results.append(JSON.validate_python({"flow": flow, "rule": rule}))
    return JSON.validate_python({"results": results})


async def flow_backfill_op(call: dict[str, JsonValue]) -> JsonValue:
    table = ServerTable([row_of(spec) for spec in DICTS.validate_python(call["flow_rows"])])
    credentials = CredentialTable([row_of(spec) for spec in DICTS.validate_python(call["user_credentials"])])
    counts = await backfill_null_oauth2_flows(FakeClient(table, credentials))
    stamps = [{"flow": flow, "server_ids": sorted(ids)} for flow, ids in table.stamps]
    rows = [
        JSON.validate_python({"server_id": row.server_id, "oauth2_flow": row.oauth2_flow, "updated_by": row.updated_by})
        for row in sorted(table.rows)
    ]
    ambiguous = sorted(row.server_id for row in table.rows if row.oauth2_flow is None)
    return JSON.validate_python({"counts": dict(counts), "rows": rows, "stamps": stamps, "ambiguous": ambiguous})


async def issuer_backfill_op(call: dict[str, JsonValue]) -> JsonValue:
    table = IssuerTable(
        [row_of(spec) for spec in DICTS.validate_python(call["issuer_rows"])],
        TypeAdapter(list[str]).validate_python(call["fail_server_ids"]),
    )
    healed = 0
    for _run in range(INT.validate_python(call["runs"])):
        healed += await backfill_discovery_stamped_issuers(FakeClient(table, CredentialTable([])))
    rows = [
        JSON.validate_python({"server_id": row.server_id, "issuer": row.issuer, "updated_by": row.updated_by})
        for row in sorted(table.rows)
    ]
    failed = sorted(server_id for server_id in table.attempted if server_id not in table.updated)
    return JSON.validate_python(
        {"healed": healed, "rows": rows, "updated": sorted(table.updated), "failed": failed}
    )


async def resolve_op(call: dict[str, JsonValue]) -> JsonValue:
    client = ScriptedClient(DICTS.validate_python(call["outcomes"]))
    server = SERVER.validate_python(call["server"])
    auth = await resolve_mcp_auth(server, OVERRIDE.validate_python(call["mcp_auth_header"]))
    return JSON.validate_python({"auth": auth, "posts": client.posts})


async def header_op(call: dict[str, JsonValue]) -> JsonValue:
    server = SERVER.validate_python(call["server"])
    header = resolved_token_header(server, OVERRIDE.validate_python(call["mcp_auth_header"]))
    return JSON.validate_python({"header": header})


OPS: dict[str, Callable[[dict[str, JsonValue]], Awaitable[JsonValue]]] = {
    "identity": identity_op, "oauth2_get_token": get_token_op, "oauth2_fetch": fetch_op,
    "oauth2_invalidate": invalidate_op, "per_user_get": per_user_get_op, "per_user_set": per_user_set_op,
    "per_user_delete": per_user_delete_op, "per_user_ttl": per_user_ttl_op, "flow_classify": classify_op,
    "flow_backfill": flow_backfill_op, "issuer_backfill": issuer_backfill_op, "auth_resolve": resolve_op,
    "token_header": header_op,
}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    operation = STR.validate_python(input_value["function"])
    return asyncio.run(OPS[operation](input_value))
