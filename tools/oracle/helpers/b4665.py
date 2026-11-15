import asyncio
from typing import Literal, NotRequired, TypedDict, cast

from litellm.constants import (
    MCP_OAUTH2_TOKEN_CACHE_DEFAULT_TTL as TOKEN_DEFAULT_TTL,
)
from litellm.constants import (
    MCP_OAUTH2_TOKEN_CACHE_MIN_TTL as TOKEN_MIN_TTL,
)
from litellm.constants import (
    MCP_OAUTH2_TOKEN_EXPIRY_BUFFER_SECONDS as TOKEN_BUFFER,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials import (
    CredError,
    Error,
    Ok,
    Result,
    ServerSpec,
    TokenExchangeConfig,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.oauth_token_store import (
    InMemoryTokenCacheBackend,
    OAuthToken,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.token_exchanger import (
    ExchangeHttpPost,
    OboTokenExchanger,
    SubjectTokenRejected,
    TokenExchangeClientError,
)
from litellm.types.mcp import DEFAULT_SUBJECT_TOKEN_TYPE
from pydantic import JsonValue as Json
from pydantic import SecretStr

DEFAULT_SERVER_ID = "srv"
DEFAULT_CALLERS = 2
DEFAULT_CACHE_SIZE = 4096


class ConfigSpec(TypedDict):
    profile: NotRequired[Literal["rfc8693", "entra_obo"] | None]
    subject_token_type: NotRequired[str | None]
    token_exchange_endpoint: NotRequired[str | None]
    audience: NotRequired[str | None]
    client_id: NotRequired[str | None]
    client_secret: NotRequired[str | None]
    token_endpoint_auth_method: NotRequired[
        Literal["client_secret_basic", "client_secret_post"] | None
    ]
    scopes: NotRequired[list[str] | None]


class ResponseSpec(TypedDict):
    kind: Literal["body", "transport", "subject_rejected", "client_fault"]
    body: NotRequired[dict[str, Json]]
    detail: NotRequired[str | None]
    claims: NotRequired[str | None]


class StepSpec(TypedDict):
    op: Literal["exchange", "invalidate", "concurrent"]
    subject_token: str
    tenant_id: NotRequired[str | None]
    now_ms: NotRequired[int | None]
    config: NotRequired[ConfigSpec]
    responses: NotRequired[list[ResponseSpec]]
    count: NotRequired[int | None]


class Input(TypedDict):
    config: ConfigSpec
    default_ttl_ms: NotRequired[int | None]
    min_ttl_ms: NotRequired[int | None]
    expiry_buffer_ms: NotRequired[int | None]
    steps: list[StepSpec]


def build_config(spec: ConfigSpec) -> TokenExchangeConfig:
    profile = spec.get("profile")
    subject_type = spec.get("subject_token_type")
    secret = spec.get("client_secret")
    scopes = spec.get("scopes")
    return TokenExchangeConfig(
        profile="rfc8693" if profile is None else profile,
        subject_token_type=DEFAULT_SUBJECT_TOKEN_TYPE if subject_type is None else subject_type,
        token_exchange_endpoint=spec.get("token_exchange_endpoint"),
        audience=spec.get("audience"),
        client_id=spec.get("client_id"),
        client_secret=None if secret is None else SecretStr(secret),
        token_endpoint_auth_method=spec.get("token_endpoint_auth_method"),
        scopes=() if scopes is None else tuple(scopes),
    )


class ManualClock:
    def __init__(self) -> None:
        self.value = 0.0

    def __call__(self) -> float:
        return self.value


class RecordingCache:
    def __init__(self, clock: ManualClock) -> None:
        self.clock = clock
        self.inner = InMemoryTokenCacheBackend(max_size=DEFAULT_CACHE_SIZE, clock=clock)
        self.events: list[Json] = []

    async def get(self, user_id: str, server_id: str) -> OAuthToken | None:
        read = self.inner.get
        token = await read(user_id, server_id)
        self.events.append(
            {"op": "get", "key": user_id, "server": server_id, "hit": token is not None}
        )
        return token

    async def set(
        self, user_id: str, server_id: str, token: OAuthToken, ttl_seconds: float
    ) -> None:
        self.events.append(
            {
                "op": "set",
                "key": user_id,
                "server": server_id,
                "ttl_ms": round(ttl_seconds * 1000),
            }
        )
        await self.inner.set(user_id, server_id, token, ttl_seconds)

    async def delete(self, user_id: str, server_id: str) -> None:
        self.events.append({"op": "delete", "key": user_id, "server": server_id})
        await self.inner.delete(user_id, server_id)


def call_view(url: str, form: dict[str, str], headers: dict[str, str]) -> Json:
    return {
        "url": url,
        "form": {**form},
        "headers": {**headers},
    }


class ScriptedPost:
    def __init__(self) -> None:
        self.responses: list[ResponseSpec] = []
        self.calls: list[Json] = []

    async def __call__(
        self, url: str, form: dict[str, str], headers: dict[str, str]
    ) -> dict[str, Json] | None:
        self.calls.append(call_view(url, form, headers))
        await asyncio.sleep(0)
        outcome: ResponseSpec = {"kind": "transport"}
        if self.responses:
            outcome = self.responses.pop(0)
        kind = outcome["kind"]
        if kind == "body":
            body = outcome.get("body")
            return None if body is None else dict(body.items())
        if kind == "subject_rejected":
            detail = outcome.get("detail")
            claims = outcome.get("claims")
            raise SubjectTokenRejected("" if detail is None else detail, claims=claims)
        if kind == "client_fault":
            raise TokenExchangeClientError("invalid_client")
        return None


def seconds_or(milliseconds: int | None, default: int) -> float:
    if milliseconds is None:
        return float(default)
    return milliseconds / 1000


def build_exchanger(
    input_value: Input,
    post: ScriptedPost,
    cache: RecordingCache,
    clock: ManualClock,
) -> OboTokenExchanger:
    http_post = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/outbound_credentials/token_exchanger.py#L80
        ExchangeHttpPost,
        post,
    )
    return OboTokenExchanger(
        http_post,
        cache=cache,
        clock=clock,
        default_ttl_seconds=seconds_or(input_value.get("default_ttl_ms"), TOKEN_DEFAULT_TTL),
        min_ttl_seconds=seconds_or(input_value.get("min_ttl_ms"), TOKEN_MIN_TTL),
        expiry_buffer_seconds=seconds_or(input_value.get("expiry_buffer_ms"), TOKEN_BUFFER),
    )


def token_view(token: OAuthToken) -> Json:
    expires_at = token.expires_at
    return {
        "access_token": token.access_token,
        "expires_at_ms": None if expires_at is None else round(expires_at * 1000),
        "refresh_token": token.refresh_token,
        "scopes": list(token.scopes),
        "identity_binding_proof": token.identity_binding_proof,
    }


def error_view(error: CredError) -> Json:
    claims = error.unauthorized.claims if error.tag == "unauthorized" else None
    return {"kind": "error", "tag": error.tag, "claims": claims}


def result_view(result: Result[OAuthToken, CredError]) -> Json:
    match result:
        case Ok(token):
            return {"kind": "ok", "token": token_view(token)}
        case Error(error):
            return error_view(error)
    raise ValueError("the result is neither Ok nor Error")


class Scenario:
    def __init__(self, input_value: Input) -> None:
        self.input = input_value
        self.clock = ManualClock()
        self.cache = RecordingCache(self.clock)
        self.post = ScriptedPost()
        self.exchanger = build_exchanger(input_value, self.post, self.cache, self.clock)

    async def run_concurrent(
        self,
        step: StepSpec,
        server: ServerSpec,
        config: TokenExchangeConfig,
        tenant: str,
    ) -> Json:
        count = step.get("count")
        callers = DEFAULT_CALLERS if count is None else count
        pending = [
            self.exchanger.exchange(step["subject_token"], server, config, tenant_id=tenant)
            for _ in range(callers)
        ]
        results = await asyncio.gather(*pending)
        return {
            "op": "concurrent",
            "results": [result_view(result) for result in results],
            "posts": len(self.post.calls),
        }

    async def run_step(self, step: StepSpec) -> Json:
        now_ms = step.get("now_ms")
        if now_ms is not None:
            self.clock.value = now_ms / 1000
        step_config = step.get("config")
        config = build_config(self.input["config"] if step_config is None else step_config)
        server = ServerSpec(server_id=DEFAULT_SERVER_ID, resource="", config=config)
        tenant_id = step.get("tenant_id")
        tenant = "" if tenant_id is None else tenant_id
        responses = step.get("responses")
        self.post.responses = [] if responses is None else list(responses)
        self.post.calls.clear()
        self.cache.events.clear()
        op = step["op"]
        if op == "concurrent":
            return await self.run_concurrent(step, server, config, tenant)
        result: Json
        if op == "exchange":
            exchanged = await self.exchanger.exchange(
                step["subject_token"], server, config, tenant_id=tenant
            )
            result = result_view(exchanged)
        else:
            await self.exchanger.invalidate(
                step["subject_token"], server, config, tenant_id=tenant
            )
            result = {"kind": "none"}
        return {
            "op": op,
            "result": result,
            "events": list(self.cache.events),
            "calls": list(self.post.calls),
        }

    async def run_steps(self) -> list[Json]:
        steps: list[Json] = [await self.run_step(step) for step in self.input["steps"]]
        return steps


def run(input_value: Input) -> Json:
    scenario = Scenario(input_value)
    steps = asyncio.run(scenario.run_steps())
    return {"steps": steps}
