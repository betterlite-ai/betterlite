import asyncio
import os
from collections import UserDict
from typing import Final, TypedDict
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
    _compute_per_user_token_ttl,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth2_token_cache.py#L206-L221
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

Where = dict[str, JsonValue]
STRING: Final = TypeAdapter(str)
OPTIONAL_STRING: Final = TypeAdapter(str | None)
JSON_VALUE: Final = TypeAdapter(JsonValue)


class Input(TypedDict):
    function: str
    servers: list[Where]
    server: Where
    outcomes: list[Where]
    calls: int
    concurrent: bool
    user_id: str
    server_id: str
    seed: str | None
    garbage: bool
    access_token: str
    ttl: int
    identity_binding_proof: str | None
    expires_in: int | None
    fail: str | None
    signals: list[Where]
    flow_rows: list[Where]
    user_credentials: list[Where]
    issuer_rows: list[Where]
    fail_server_ids: list[str]
    runs: int
    mcp_auth_header: str | dict[str, str] | None


class ScriptedClient(LLMClientCache):
    def __init__(self, outcomes: list[Where]) -> None:
        super().__init__()
        self.outcomes, self.posts = outcomes, 0
        self.form: list[list[str]] = []
        self.authorization: str | None = None
        self.handler = AsyncHTTPHandler(transport=httpx.MockTransport(self.respond))
        litellm.in_memory_llm_clients_cache = self

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> AsyncHTTPHandler: return self.handler

    def respond(self, request: httpx.Request) -> httpx.Response:
        self.posts += 1
        self.form = [[name, value] for name, value in sorted(parse_qsl(request.content.decode()))]
        self.authorization = request.headers.get("authorization")
        body = JSON_VALUE.validate_json(STRING.validate_python(self.outcomes.pop(0)["body"]))
        return httpx.Response(200, json=body, request=request)


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
        self.ttls[key] = TypeAdapter(int).validate_python(kwargs["ttl"])
    @override
    def delete_cache(self, key: str) -> None:
        if self.failure == "delete":
            raise RuntimeError("the fake cache delete failed")
        self.raw.pop(key, None)


class FakeRow(UserDict[str, JsonValue]):
    # LiteLLM reads a row's columns as attributes; a column a table does not carry reads as None.
    def __getattr__(self, name: str) -> JsonValue:
        return self.get(name)

    def by_server_id(self) -> str:
        return STRING.validate_python(self["server_id"])


class FakeTable:
    def __init__(self, rows: list[Where], match: tuple[str, JsonValue] | None = None) -> None:
        self.rows = [FakeRow(spec) for spec in rows]
        self.match = match
        self.fail: list[str] = []
        self.stamps: list[tuple[str, list[str]]] = []
        self.attempted: list[str] = []
        self.updated: list[str] = []

    async def find_many(self, where: Where) -> list[FakeRow]:
        if self.match is not None:
            return [row for row in self.rows if row.get(self.match[0]) == self.match[1]]
        ids = TypeAdapter(dict[str, list[str]]).validate_python(where["server_id"])["in"]
        return [row for row in self.rows if STRING.validate_python(row["server_id"]) in ids]

    async def update_many(self, *, where: Where, data: Where) -> None:
        ids = TypeAdapter(dict[str, list[str]]).validate_python(where["server_id"])["in"]
        self.stamps.append((STRING.validate_python(data["oauth2_flow"]), sorted(ids)))
        for server_id in ids:
            await self.update(where={"server_id": server_id}, data=data)

    async def update(self, *, where: Where, data: Where) -> None:
        server_id = STRING.validate_python(where["server_id"])
        self.attempted.append(server_id)
        if server_id in self.fail:
            raise RuntimeError("the fake issuer write failed")
        for row in self.rows:
            if STRING.validate_python(row["server_id"]) == server_id:
                row.update(data)
        self.updated.append(server_id)


class FakeDb(PrismaWrapper):
    def __init__(self, servers: FakeTable, credentials: FakeTable) -> None:
        self.litellm_mcpservertable = servers
        self.litellm_mcpusercredentials = credentials


class FakeClient(PrismaClient):
    def __init__(self, servers: FakeTable, credentials: FakeTable) -> None:
        self.db = FakeDb(servers, credentials)


def row_views(rows: list[FakeRow], field: str) -> list[Where]:
    return [
        {"server_id": row["server_id"], field: row.get(field), "updated_by": row.get("updated_by")}
        for row in sorted(rows, key=FakeRow.by_server_id)
    ]


async def stored_token(cache: MCPPerUserTokenCache, user_id: str, server_id: str) -> str | None:
    token = await cache.get_token(user_id, server_id)
    return None if token is None else token.access_token


async def oauth2(call: Input) -> JsonValue:
    if call["function"] == "identity":
        identities = [
            MCPOAuth2TokenCache._token_identity(server)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth2_token_cache.py#L66-L81
            for server in TypeAdapter(list[MCPServer]).validate_python(call["servers"])
        ]
        return {"identities": list[JsonValue](identities)}
    server = TypeAdapter(MCPServer).validate_python(call["server"])
    if call["function"] == "token_header":
        return {"header": resolved_token_header(server, call.get("mcp_auth_header"))}
    client = ScriptedClient(call["outcomes"])
    if call["function"] == "auth_resolve":
        auth = await resolve_mcp_auth(server, call.get("mcp_auth_header"))
        return {"auth": JSON_VALUE.validate_python(auth), "posts": client.posts}
    cache = MCPOAuth2TokenCache()
    if call["function"] == "oauth2_invalidate":
        first = await cache.async_get_token(server)
        cache.invalidate(server.server_id)
        second = await cache.async_get_token(server)
        return {"tokens": [first, second], "posts": client.posts}
    if call["function"] == "oauth2_fetch":
        token, ttl = await cache._fetch_token(server)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth2_token_cache.py#L117-L190
        return {
            "token": token, "ttl": ttl, "form": [[name, value] for name, value in client.form],
            "authorization": client.authorization,
        }
    calls = range(call["calls"])
    if call.get("concurrent"):
        tokens = list(await asyncio.gather(*[cache.async_get_token(server) for _ in calls]))
    else:
        tokens = [await cache.async_get_token(server) for _ in calls]
    return {"tokens": list[JsonValue](tokens), "posts": client.posts}


async def per_user(call: Input) -> JsonValue:
    action = call["function"].removeprefix("per_user_")
    if action == "ttl":
        server = TypeAdapter(MCPServer).validate_python(call["server"])
        ttl = _compute_per_user_token_ttl(server, call.get("expires_in"))
        return {"ttl": ttl}
    fail = call.get("fail") is not None
    memory = Memory(action if fail else "none")
    os.environ["LITELLM_SALT_KEY"] = "b4662-oracle-salt"
    proxy_server.redis_usage_cache = None
    proxy_server.user_api_key_cache = UserApiKeyCache(in_memory_cache=memory, redis_cache=None)
    cache = MCPPerUserTokenCache()
    user_id, server_id = call["user_id"], call["server_id"]
    if action == "get":
        seed = call.get("seed")
        if seed is not None:
            await cache.set(user_id, server_id, seed, 300)
        if call.get("garbage"):
            memory.raw[cache._cache_key(user_id, server_id)] = "garbage"  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth2_token_cache.py#L231-L232
        return {"token": await stored_token(cache, user_id, server_id)}
    if action == "set":
        access_token = call["access_token"]
        proof = call.get("identity_binding_proof")
        await cache.set(user_id, server_id, access_token, call["ttl"], proof)
        stored = next(iter(memory.raw.values()), None)
        return {
            "token": await stored_token(cache, user_id, server_id),
            "raw_differs": stored is not None and stored != access_token,
            "ttl": next(iter(memory.ttls.values()), None),
        }
    await cache.set(user_id, server_id, "token-value", 300)
    await cache.delete(user_id, server_id)
    return {"token_after": await stored_token(cache, user_id, server_id)}


async def classify(call: Input) -> JsonValue:
    results: list[Where] = []
    for signal in call["signals"]:
        credentials = TypeAdapter(MCPCredentials | None).validate_python(signal.get("credentials"))
        flow, rule = classify_null_flow_row(
            has_per_user_tokens=TypeAdapter(bool).validate_python(signal["has_per_user_tokens"]),
            authorization_url=OPTIONAL_STRING.validate_python(signal.get("authorization_url")),
            registration_url=OPTIONAL_STRING.validate_python(signal.get("registration_url")),
            token_url=OPTIONAL_STRING.validate_python(signal.get("token_url")),
            credentials=credentials,
        )
        results.append({"flow": flow, "rule": rule})
    return {"results": list[JsonValue](results)}


async def flow_backfill(call: Input) -> JsonValue:
    table = FakeTable(call["flow_rows"], ("oauth2_flow", None))
    tally = await backfill_null_oauth2_flows(FakeClient(table, FakeTable(call["user_credentials"])))
    open_rows = [row for row in table.rows if row.get("oauth2_flow") is None]
    ambiguous = sorted(STRING.validate_python(row["server_id"]) for row in open_rows)
    counts: dict[str, JsonValue] = dict[str, JsonValue](tally.items())
    return {
        "counts": counts,
        "rows": list[JsonValue](row_views(table.rows, "oauth2_flow")),
        "stamps": [
            {"flow": flow, "server_ids": list[JsonValue](sorted(ids))} for flow, ids in table.stamps
        ],
        "ambiguous": list[JsonValue](ambiguous),
    }


async def issuer_backfill(call: Input) -> JsonValue:
    table = FakeTable(call["issuer_rows"], ("updated_by", "mcp_oauth_discovery"))
    table.fail = call["fail_server_ids"]
    healed = 0
    for _run in range(call["runs"]):
        healed += await backfill_discovery_stamped_issuers(FakeClient(table, FakeTable([])))
    return {
        "healed": healed,
        "rows": list[JsonValue](row_views(table.rows, "issuer")),
        "updated": list[JsonValue](sorted(table.updated)),
        "failed": list[JsonValue](sorted(i for i in table.attempted if i not in table.updated)),
    }


OPS = {
    "identity": oauth2, "oauth2_get_token": oauth2, "oauth2_fetch": oauth2, "auth_resolve": oauth2,
    "oauth2_invalidate": oauth2, "token_header": oauth2, "issuer_backfill": issuer_backfill,
    "per_user_get": per_user, "per_user_set": per_user, "per_user_delete": per_user,
    "per_user_ttl": per_user, "flow_classify": classify, "flow_backfill": flow_backfill,
}


def run(input_value: Input) -> JsonValue:
    handler = OPS[input_value["function"]]
    return asyncio.run(handler(input_value))
