import asyncio
from collections.abc import Awaitable, Mapping
from typing import Literal, NotRequired, TypeAlias, TypedDict, TypeVar

from litellm.proxy._experimental.mcp_server.outbound_credentials.dual_cache_token_backend import (
    DualCacheTokenCacheBackend,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.oauth_token_store import (
    CachedOAuthTokenStore,
    InMemoryTokenCacheBackend,
    InProcessRefreshCoordinator,
    OAuthToken,
    RefreshingTokenStore,
    TokenStoreUnavailable,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.presented_token_store import (
    PresentedOAuthTokenStore,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.token_cache_codec import (
    OAuthTokenCacheCodec,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.v2_token_store import (
    V2PerUserTokenStore,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
T = TypeVar("T")


class TokenSpec(TypedDict):
    access_token: str
    expires_at_ms: int | None
    refresh_token: str | None
    scopes: list[str]
    identity_binding_proof: str | None


class OutcomeSpec(TypedDict):
    kind: Literal["token", "absent", "unavailable"]
    token: NotRequired[TokenSpec]


class Operation(TypedDict):
    user_id: str
    server_id: str
    op: NotRequired[Literal["get", "set", "delete", "fetch", "invalidate"]]
    now_ms: NotRequired[int]
    ttl_ms: NotRequired[int]
    token: NotRequired[TokenSpec]


class Input(TypedDict):
    function: str
    kind: NotRequired[Literal["cache", "cached", "refreshing", "dual", "v2"]]
    token: NotRequired[TokenSpec]
    operations: NotRequired[list[Operation]]
    max_size: NotRequired[int]
    inner: NotRequired[list[OutcomeSpec]]
    refresher: NotRequired[list[OutcomeSpec]]
    cache_get: NotRequired[Literal["missing", "stored", "error"]]
    set_error: NotRequired[bool]
    delete_error: NotRequired[bool]
    operation: NotRequired[Literal["encode", "decode"]]
    plaintext: NotRequired[str]
    refresh: NotRequired[OutcomeSpec]
    payloads: NotRequired[list[dict[str, Json] | None]]


def required(value: T | None, name: str) -> T:
    if value is None:
        raise ValueError(f"{name} is required")
    return value


def build_token(spec: TokenSpec) -> OAuthToken:
    return OAuthToken(
        access_token=spec["access_token"],
        expires_at=None if spec["expires_at_ms"] is None else spec["expires_at_ms"] / 1000,
        refresh_token=spec["refresh_token"], scopes=tuple(spec["scopes"]),
        identity_binding_proof=spec["identity_binding_proof"],
    )


def token_view(token: OAuthToken) -> Json:
    expires_at = token.expires_at
    expires_at_ms = None if expires_at is None else int(expires_at * 1000)
    return {"access_token": token.access_token, "expires_at_ms": expires_at_ms,
            "refresh_token": token.refresh_token, "scopes": list(token.scopes),
            "identity_binding_proof": token.identity_binding_proof}


async def captured(awaitable: Awaitable[OAuthToken | None]) -> Json:
    try:
        token = await awaitable
    except TokenStoreUnavailable:
        return {"error": "unavailable"}
    return None if token is None else token_view(token)


class ManualClock:
    def __init__(self) -> None:
        self.value = 0.0

    def __call__(self) -> float:
        return self.value


class ScriptedStore:
    def __init__(self, outcomes: list[OutcomeSpec]) -> None:
        self.outcomes = list(outcomes)
        self.calls = self.reread_calls = 0

    def next(self) -> OAuthToken | None:
        self.calls += 1
        if not self.outcomes:
            raise TokenStoreUnavailable
        outcome = self.outcomes.pop(0)
        if outcome["kind"] == "absent":
            return None
        if outcome["kind"] == "unavailable":
            raise TokenStoreUnavailable
        return build_token(required(outcome.get("token"), "token"))

    async def next_async(self) -> OAuthToken | None:
        await asyncio.sleep(0)
        return self.next()

    async def fetch(self, user_id: str, server_id: str) -> OAuthToken | None:
        del user_id, server_id
        return await self.next_async()

    async def refresh(self, user_id: str, server_id: str, token: OAuthToken) -> OAuthToken | None:
        del user_id, server_id, token
        return await self.next_async()

    async def reread(self) -> OAuthToken | None:
        self.reread_calls += 1
        return None


class FakeCodec:
    def encrypt(self, value: str) -> str:
        if not value:
            return "enc:"
        return f"enc:{value}"

    def decrypt(self, blob: str) -> str | None:
        if not blob.startswith("enc:"):
            return None
        return blob[len("enc:") :]


class FakeCache:
    def __init__(self, kind: str | None, *, set_error: bool, delete_error: bool) -> None:
        self.kind = kind
        self.set_error, self.delete_error = set_error, delete_error
        self.stored: dict[str, str] = {}

    async def async_get_cache(self, key: str) -> str | None:
        if self.kind == "error":
            raise RuntimeError
        if self.kind == "stored":
            return self.stored.get(key)
        return None

    async def async_set_cache(self, key: str, value: str, ttl: float | None = None) -> None:
        del ttl
        if self.set_error:
            raise RuntimeError
        self.stored[key] = value

    async def async_delete_cache(self, key: str) -> None:
        if self.delete_error:
            raise RuntimeError
        self.stored.pop(key, None)


class PayloadReader:
    def __init__(self, payloads: list[dict[str, Json] | None]) -> None:
        self.payloads = list(payloads)

    async def read(self, user_id: str, server_id: str) -> Mapping[str, Json] | None:
        del user_id, server_id
        return self.payloads.pop(0) if self.payloads else None


def token(input_value: Input) -> Json:
    value = build_token(required(input_value.get("token"), "token"))
    rendered = repr(value)
    fetched = asyncio.run(PresentedOAuthTokenStore(value).fetch("user", "server"))
    return {"token": token_view(value), "access_token_in_debug": value.access_token in rendered,
            "refresh_token_in_debug":
                value.refresh_token is not None and value.refresh_token in rendered,
            "has_refresh_token_marker": "has_refresh_token=True" in rendered,
            "presented": None if fetched is None else token_view(fetched)}


def codec(input_value: Input) -> Json:
    codec = OAuthTokenCacheCodec(encrypt=FakeCodec().encrypt, decrypt=FakeCodec().decrypt)
    operation = required(input_value.get("operation"), "operation")
    if operation == "encode":
        encoded = codec.encode(build_token(required(input_value.get("token"), "token")))
        plaintext = codec.decrypt(encoded)
        decoded = codec.decode(encoded)
        return {"bound": plaintext is not None and plaintext.startswith("litellm-bound-oauth-v1:"),
                "decoded": None if decoded is None else token_view(decoded)}
    decoded = codec.decode(codec.encrypt(required(input_value.get("plaintext"), "plaintext")))
    return {"decoded": None if decoded is None else token_view(decoded)}


async def cache_steps(
    cache: InMemoryTokenCacheBackend | DualCacheTokenCacheBackend,
    input_value: Input,
    clock: ManualClock,
) -> list[Json]:
    results: list[Json] = []
    get_token = cache.get
    for operation in required(input_value.get("operations"), "operations"):
        clock.value = required(operation.get("now_ms"), "now_ms") / 1000
        if operation.get("op") == "set":
            token = build_token(required(operation.get("token"), "token"))
            ttl = required(operation.get("ttl_ms"), "ttl_ms") / 1000
            await cache.set(operation["user_id"], operation["server_id"], token, ttl)
            results.append({"result": None})
        elif operation.get("op") == "delete":
            await cache.delete(operation["user_id"], operation["server_id"])
            results.append({"result": None})
        else:
            result = await captured(get_token(operation["user_id"], operation["server_id"]))
            results.append({"result": result})
    return results


async def run_store(input_value: Input) -> Json:
    kind = required(input_value.get("kind"), "kind")
    clock = ManualClock()
    maximum = input_value.get("max_size")
    max_size = 4096 if maximum is None else maximum
    if kind == "cache":
        memory = InMemoryTokenCacheBackend(max_size=max_size, clock=clock)
        return {"operations": await cache_steps(memory, input_value, clock)}
    if kind == "v2":
        reader = PayloadReader(required(input_value.get("payloads"), "payloads"))
        store = V2PerUserTokenStore(reader.read)
        count = len(reader.payloads)
        fetched = [await captured(store.fetch("user", "server")) for _ in range(count)]
        return {"results": fetched}
    if kind == "dual":
        dual = DualCacheTokenCacheBackend(
            cache=FakeCache(input_value.get("cache_get"),
                            set_error=input_value.get("set_error") is True,
                            delete_error=input_value.get("delete_error") is True),
            codec=OAuthTokenCacheCodec(encrypt=FakeCodec().encrypt, decrypt=FakeCodec().decrypt),
            key_prefix="mcp:per_user_token:",
        )
        return {"operations": await cache_steps(dual, input_value, clock)}
    source = ScriptedStore(required(input_value.get("inner"), "inner"))
    if kind == "cached":
        cached = CachedOAuthTokenStore(
            source, default_ttl_seconds=300.0,
            backend=InMemoryTokenCacheBackend(max_size=max_size, clock=clock), clock=clock,
        )
        reads: list[Json] = []
        for operation in required(input_value.get("operations"), "operations"):
            clock.value = required(operation.get("now_ms"), "now_ms") / 1000
            result = await captured(cached.fetch(operation["user_id"], operation["server_id"]))
            reads.append({"result": result, "inner_reads": source.calls})
        return {"operations": reads}
    refresher = ScriptedStore(required(input_value.get("refresher"), "refresher"))
    refreshing = RefreshingTokenStore(source, refresher, clock=clock)
    updates: list[Json] = []
    for operation in required(input_value.get("operations"), "operations"):
        clock.value = required(operation.get("now_ms"), "now_ms") / 1000
        result = await captured(refreshing.fetch(operation["user_id"], operation["server_id"]))
        updates.append({"result": result, "inner_reads": source.calls,
                        "refresh_calls": refresher.calls})
    return {"operations": updates}


async def run_coordinator(input_value: Input) -> Json:
    script = ScriptedStore([required(input_value.get("refresh"), "refresh")])
    flight = InProcessRefreshCoordinator()
    refresh, reread = script.next_async, script.reread
    first = asyncio.ensure_future(flight.run("user", "server", refresh=refresh, reread=reread))
    second = asyncio.ensure_future(flight.run("user", "server", refresh=refresh, reread=reread))
    third = asyncio.ensure_future(flight.run("user", "server", refresh=refresh, reread=reread))
    tokens = [await first, await second, await third]
    return {"results": [None if token is None else token_view(token) for token in tokens],
            "refresh_calls": script.calls, "reread_calls": script.reread_calls}


def run(input_value: Input) -> Json:
    function = input_value["function"]
    if function == "token":
        return token(input_value)
    if function == "codec":
        return codec(input_value)
    if function == "coordinator":
        return asyncio.run(run_coordinator(input_value))
    return asyncio.run(run_store(input_value))
