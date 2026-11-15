import asyncio
from typing import Literal, NotRequired, TypedDict, TypeVar
from urllib.parse import parse_qsl

import httpx
from aiohttp import ClientSession
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._experimental.mcp_server.outbound_credentials import token_endpoint
from litellm.proxy._experimental.mcp_server.outbound_credentials.oauth_token_store import (
    OAuthToken,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.redis_refresh_coordinator import (
    LockAcquisition,
    RedisRefreshCoordinator,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.result import (
    Error,
    Ok,
    Result,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.token_endpoint import (
    ExchangedToken,
    ExchangedTokenCache,
    TokenEndpointClient,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.types import (
    ClientAuth,
    ClientSecretAuth,
    CredError,
    PrivateKeyJwtAuth,
)
from litellm.types.llms.custom_http import httpxSpecialProvider
from litellm.types.utils import LlmProviders
from pydantic import JsonValue as Json
from pydantic import SecretStr

T = TypeVar("T")
HEX_LENGTH = 32


class AuthSpec(TypedDict):
    source: Literal["client_secret", "private_key_jwt"]
    client_secret: NotRequired[str]
    private_key: NotRequired[str]
    key_id: NotRequired[str | None]
    signing_alg: NotRequired[str]


class ResponseSpec(TypedDict):
    kind: Literal["answered", "unreachable"]
    status: NotRequired[int]
    body: NotRequired[str]


class ComputeSpec(TypedDict):
    kind: Literal["token", "error"]
    access_token: NotRequired[str]
    expires_in: NotRequired[int | None]
    detail: NotRequired[str]


class OperationSpec(TypedDict):
    op: Literal["get_or_compute", "invalidate"]
    key: str
    fingerprint: NotRequired[str]
    compute: NotRequired[ComputeSpec]


class Input(TypedDict):
    function: str
    endpoint: NotRequired[str]
    client_id: NotRequired[str]
    grant: NotRequired[dict[str, str]]
    client_auth: NotRequired[AuthSpec]
    response: NotRequired[ResponseSpec]
    operations: NotRequired[list[OperationSpec]]


def required(value: T | None, name: str) -> T:
    if value is None:
        raise ValueError(f"{name} is required")
    return value


class ScriptedEndpoint:
    def __init__(self, spec: ResponseSpec) -> None:
        self.spec = spec
        self.contents: list[bytes] = []

    def client(
        self,
        llm_provider: LlmProviders | httpxSpecialProvider | str,
        params: dict[str, str] | None = None,
        shared_session: ClientSession | None = None,
    ) -> AsyncHTTPHandler:
        del llm_provider, params, shared_session
        return AsyncHTTPHandler(transport=httpx.MockTransport(self.handle))

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.contents.append(request.content)
        if self.spec["kind"] == "unreachable":
            raise httpx.ConnectError(f"token endpoint {self.spec['kind']}")
        status = required(self.spec.get("status"), "status")
        body = required(self.spec.get("body"), "body")
        return httpx.Response(status_code=status, content=body.encode(), request=request)


def form_view(content: bytes) -> Json:
    pairs = parse_qsl(content.decode(), keep_blank_values=True)
    rendered = [
        [name, "<assertion>" if name == "client_assertion" else value]
        for name, value in pairs
    ]
    return [list(pair) for pair in sorted(rendered)]


def auth_client(spec: AuthSpec) -> ClientAuth:
    if spec["source"] == "client_secret":
        return ClientSecretAuth(
            client_secret=SecretStr(required(spec.get("client_secret"), "client_secret")),
        )
    return PrivateKeyJwtAuth(
        private_key=SecretStr(required(spec.get("private_key"), "private_key")),
        key_id=spec.get("key_id"),
        signing_alg=required(spec.get("signing_alg"), "signing_alg"),
    )


def token_outcome(result: Result[ExchangedToken, CredError]) -> Json:
    match result:
        case Ok(token):
            return {"ok": True, "access_token": token.access_token, "expires_in": token.expires_in}
        case Error(error):
            return {"ok": False, "error": error.tag}


def fetch(input_value: Input) -> Json:
    endpoint = required(input_value.get("endpoint"), "endpoint")
    client_id = required(input_value.get("client_id"), "client_id")
    grant = required(input_value.get("grant"), "grant")
    auth = auth_client(required(input_value.get("client_auth"), "client_auth"))
    script = ScriptedEndpoint(required(input_value.get("response"), "response"))
    token_endpoint.get_async_httpx_client = script.client  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L1713
    result = asyncio.run(TokenEndpointClient().fetch(endpoint, client_id, grant, auth))
    if not script.contents:
        return {"result": token_outcome(result), "form": None}
    return {"result": token_outcome(result), "form": form_view(script.contents[0])}


class ScriptedCompute:
    def __init__(self, spec: ComputeSpec) -> None:
        self.spec = spec
        self.calls = 0

    async def __call__(self) -> Result[ExchangedToken, CredError]:
        self.calls += 1
        await asyncio.sleep(0)
        if self.spec["kind"] == "error":
            detail = required(self.spec.get("detail"), "detail")
            return Error(CredError.of_upstream_unavailable(detail))
        token = required(self.spec.get("access_token"), "access_token")
        return Ok(ExchangedToken(access_token=token, expires_in=self.spec.get("expires_in")))


def cache_outcome(result: Result[str, CredError]) -> Json:
    match result:
        case Ok(token):
            return {"ok": True, "token": token}
        case Error(error):
            return {"ok": False, "error": error.tag}


async def run_operations(input_value: Input) -> Json:
    cache = ExchangedTokenCache()
    results: list[Json] = []
    computes = 0
    for operation in required(input_value.get("operations"), "operations"):
        if operation["op"] == "invalidate":
            cache.invalidate(operation["key"])
            continue
        compute = ScriptedCompute(required(operation.get("compute"), "compute"))
        fingerprint = operation.get("fingerprint")
        result = await cache.get_or_compute(
            operation["key"],
            compute,
            fingerprint="" if fingerprint is None else fingerprint,
        )
        computes += compute.calls
        results.append(cache_outcome(result))
    return {"results": results, "compute_calls": computes}


class FakeLock:
    def __init__(self) -> None:
        self.held = False
        self.acquisitions: list[Json] = []
        self.releases = 0
        self.extends = 0
        self.key = ""
        self.token = ""

    async def acquire(self, key: str, token: str, ttl_seconds: float) -> LockAcquisition:
        del ttl_seconds
        self.key, self.token = key, token
        if self.held:
            self.acquisitions.append("held")
            return LockAcquisition.HELD
        self.held = True
        self.acquisitions.append("acquired")
        return LockAcquisition.ACQUIRED

    async def extend(self, key: str, token: str, ttl_seconds: float) -> bool:
        del key, token, ttl_seconds
        self.extends += 1
        return True

    async def release(self, key: str, token: str) -> None:
        del key, token
        self.releases += 1
        self.held = False

    async def is_held(self, key: str) -> bool:
        del key
        return self.held


class ScriptedTokens:
    def __init__(self) -> None:
        self.refreshes = 0
        self.rereads = 0

    async def refresh(self) -> OAuthToken | None:
        self.refreshes += 1
        await asyncio.sleep(0.05)
        return OAuthToken(access_token="fresh")  # noqa: S106

    async def reread(self) -> OAuthToken | None:
        self.rereads += 1
        return OAuthToken(access_token="reread")  # noqa: S106


async def run_case() -> Json:
    lock = FakeLock()
    tokens = ScriptedTokens()
    start = RedisRefreshCoordinator(lock, poll_interval_seconds=0.005).run
    key = ("user", "server")
    winner = asyncio.ensure_future(start(*key, refresh=tokens.refresh, reread=tokens.reread))
    await asyncio.sleep(0)
    loser = asyncio.ensure_future(start(*key, refresh=tokens.refresh, reread=tokens.reread))
    winner_token = await winner
    loser_token = await loser
    token = lock.token
    hex_32 = len(token) == HEX_LENGTH and all(letter in "0123456789abcdef" for letter in token)
    return {
        "acquisitions": lock.acquisitions,
        "refresh_calls": tokens.refreshes,
        "reread_calls": tokens.rereads,
        "release_calls": lock.releases,
        "extend_calls": lock.extends,
        "key": lock.key,
        "token_is_hex_32": hex_32,
        "winner": None if winner_token is None else winner_token.access_token,
        "loser": None if loser_token is None else loser_token.access_token,
    }


def run(input_value: Input) -> Json:
    function = input_value["function"]
    if function == "fetch":
        return fetch(input_value)
    if function == "cache":
        return asyncio.run(run_operations(input_value))
    if function == "coordinator":
        return asyncio.run(run_case())
    raise ValueError(f"unknown function {function}")
