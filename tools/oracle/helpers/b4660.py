import asyncio
from collections.abc import Mapping
from typing import Literal, NotRequired, TypedDict, TypeVar

from fastapi import HTTPException
from litellm.proxy._experimental.mcp_server.oauth_identity_binding import (
    CallerPrincipalLoader,
    RefreshOwnership,
    RefreshOwnershipProven,
    RefreshTokenPresented,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials import per_user_oauth_store
from litellm.proxy._experimental.mcp_server.outbound_credentials.authz_code_refresher import (
    AuthorizationCodeRefresher,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.oauth_token_store import (
    InvalidatableOAuthTokenStore,
    OAuthToken,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.per_user_oauth_store import (
    LazyPerUserOAuthTokenStore,
    ServerLookup,
)
from litellm.types.mcp_server.mcp_server_manager import MCPOAuthIdentityBinding, MCPServer
from pydantic import JsonValue as Json, TypeAdapter

T = TypeVar("T")


class ServerSpec(TypedDict):
    server_id: str
    name: str
    transport: str
    url: NotRequired[str]
    client_id: NotRequired[str]
    client_secret: NotRequired[str]
    token_url: NotRequired[str]
    configured_token_url: NotRequired[str]
    token_endpoint_auth_method: NotRequired[str]
    upstream_resource: NotRequired[str]
    oauth_identity_binding: NotRequired[dict[str, Json]]


class TokenSpec(TypedDict):
    access_token: str
    refresh_token: NotRequired[str]
    scopes: NotRequired[list[str]]
    identity_binding_proof: NotRequired[str]


class ValidatorSpec(TypedDict):
    outcome: Literal["proof", "none", "denied"]
    proof: NotRequired[str]


class StepSpec(TypedDict):
    op: Literal["fetch", "invalidate", "set_redis_available"]
    user_id: NotRequired[str]
    server_id: NotRequired[str]
    available: NotRequired[bool]


class ProofSpec(TypedDict):
    identity_binding_proof: NotRequired[str | None]


class Input(TypedDict):
    function: str
    server_id: NotRequired[str]
    user_id: NotRequired[str]
    server: NotRequired[ServerSpec]
    token: NotRequired[TokenSpec]
    post_answer: NotRequired[dict[str, Json]]
    validators: NotRequired[list[ValidatorSpec]]
    now: NotRequired[float]
    builds: NotRequired[list[bool]]
    binding_matches: NotRequired[list[bool]]
    steps: NotRequired[list[StepSpec]]


def required(value: T | None, name: str) -> T:
    if value is None:
        raise ValueError(f"{name} is required")
    return value


def build_server(spec: ServerSpec | None) -> MCPServer | None:
    return None if spec is None else MCPServer.model_validate(dict(spec))


def build_token(spec: TokenSpec) -> OAuthToken:
    scopes = spec.get("scopes")
    return OAuthToken(
        access_token=spec["access_token"],
        refresh_token=spec.get("refresh_token"),
        scopes=() if scopes is None else tuple(scopes),
        identity_binding_proof=spec.get("identity_binding_proof"),
    )


def token_view(token: OAuthToken) -> Json:
    expires_at = token.expires_at
    return {
        "access_token": token.access_token,
        "expires_at_millis": None if expires_at is None else int(expires_at * 1000),
        "refresh_token": token.refresh_token,
        "scopes": list(token.scopes),
        "identity_binding_proof": token.identity_binding_proof,
    }


def presented_refresh_token(ownership: RefreshOwnership) -> str | None:
    presented: str | None = None
    match ownership:
        case RefreshTokenPresented(refresh_token):
            presented = refresh_token
        case RefreshOwnershipProven() | None:
            presented = None
    return presented


class FixedLookup:
    def __init__(self, server: MCPServer | None) -> None:
        self.server = server

    def __call__(self, server_id: str) -> MCPServer | None:
        del server_id
        return self.server


class FixedClock:
    def __init__(self, now: float) -> None:
        self.now = now

    def __call__(self) -> float:
        return self.now


class ScriptedPost:
    def __init__(self, answer: dict[str, object] | None) -> None:  # ast-grep-ignore: no-loose-types
        self.answer = answer
        self.calls: list[Json] = []

    async def __call__(
        self, url: str, form: dict[str, str], headers: dict[str, str]
    ) -> dict[str, object] | None:  # ast-grep-ignore: no-loose-types
        self.calls.append({"url": url, "form": dict(form), "headers": dict(headers)})
        return self.answer


class ScriptedPersist:
    def __init__(self) -> None:
        self.calls: list[Json] = []

    async def __call__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/outbound_credentials/authz_code_refresher.py#L34-L45
        self,
        user_id: str,
        server_id: str,
        access_token: str,
        refresh_token: str | None,
        expires_in: int | None,
        scopes: tuple[str, ...] | None,
        identity_binding_proof: str | None = None,
    ) -> None:
        self.calls.append(
            {
                "user_id": user_id,
                "server_id": server_id,
                "access_token": access_token,
                "refresh_token": refresh_token,
                "expires_in": expires_in,
                "scopes": None if scopes is None else list(scopes),
                "identity_binding_proof": identity_binding_proof,
            }
        )


class ScriptedValidator:
    def __init__(self, answers: list[ValidatorSpec] | None) -> None:
        self.answers = [] if answers is None else list(answers)
        self.calls: list[Json] = []

    async def __call__(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth_identity_binding.py#L86-L95
        self,
        *,
        server: MCPServer,
        token_response: Mapping[str, object],  # ast-grep-ignore: no-loose-types
        litellm_user_id: str | None,
        grant_type: str,
        refresh_ownership: RefreshOwnership,
    ) -> str | None:
        del server
        self.calls.append(
            {
                "user_id": litellm_user_id,
                "grant_type": grant_type,
                "token_response": TypeAdapter(dict[str, Json]).validate_python(token_response),
                "refresh_token": presented_refresh_token(refresh_ownership),
            }
        )
        if not self.answers:
            return None
        answer = self.answers.pop(0)
        if answer["outcome"] == "denied":
            raise HTTPException(status_code=403, detail="oauth_identity_binding_failed")
        if answer["outcome"] == "proof":
            return required(answer.get("proof"), "proof")
        return None


async def run_refresh(input_value: Input) -> Json:
    server = build_server(input_value.get("server"))
    raw_answer = input_value.get("post_answer")
    answer: dict[str, object] | None = None  # ast-grep-ignore: no-loose-types
    if raw_answer is not None:
        adapter = TypeAdapter(dict[str, object])  # ast-grep-ignore: no-loose-types
        answer = adapter.validate_python(raw_answer)
    post = ScriptedPost(answer)
    persist = ScriptedPersist()
    validator = ScriptedValidator(input_value.get("validators"))
    refresher = AuthorizationCodeRefresher(
        FixedLookup(server),
        post,
        persist,
        clock=FixedClock(required(input_value.get("now"), "now")),
        identity_validator=validator,
    )
    token = build_token(required(input_value.get("token"), "token"))
    result = await refresher.refresh(
        required(input_value.get("user_id"), "user_id"),
        required(input_value.get("server_id"), "server_id"),
        token,
    )
    return {
        "token": None if result is None else token_view(result),
        "posts": post.calls,
        "persists": persist.calls,
        "validators": validator.calls,
    }


class ChainStore:
    def __init__(self, token: OAuthToken | None) -> None:
        self.token = token
        self.fetches = 0
        self.invalidations: list[str] = []

    async def fetch(self, user_id: str, server_id: str) -> OAuthToken | None:
        del user_id, server_id
        self.fetches += 1
        return self.token

    async def invalidate(self, user_id: str, server_id: str) -> None:
        del user_id
        self.invalidations.append(server_id)


class ScriptedBuilder:
    def __init__(self, store: ChainStore, answers: list[bool] | None) -> None:
        self.store = store
        self.answers = [] if answers is None else list(answers)
        self.builds: list[bool] = []

    def __call__(self, server_lookup: ServerLookup) -> tuple[InvalidatableOAuthTokenStore, bool]:
        del server_lookup
        uses_redis = self.answers.pop(0) if self.answers else False
        self.builds.append(uses_redis)
        return self.store, uses_redis


class ScriptedAvailability:
    def __init__(self) -> None:
        self.available = False

    def __call__(self) -> bool:
        return self.available


class ScriptedBindingMatch:
    def __init__(self, answers: list[bool] | None) -> None:
        self.answers = [] if answers is None else list(answers)
        self.calls: list[Json] = []

    async def __call__(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/oauth_identity_binding.py#L241-L252
        self,
        binding: MCPOAuthIdentityBinding,
        user_id: str,
        server_id: str,
        credential: Mapping[str, object],  # ast-grep-ignore: no-loose-types
        caller_principal_loader: CallerPrincipalLoader | None = None,
    ) -> bool:
        del binding, caller_principal_loader
        proof = TypeAdapter(ProofSpec).validate_python(credential).get("identity_binding_proof")
        self.calls.append({"user_id": user_id, "server_id": server_id, "proof": proof})
        return self.answers.pop(0) if self.answers else False


async def run_lazy(input_value: Input) -> Json:
    server = build_server(input_value.get("server"))
    spec = input_value.get("token")
    chain = ChainStore(None if spec is None else build_token(spec))
    builder = ScriptedBuilder(chain, input_value.get("builds"))
    availability = ScriptedAvailability()
    matcher = ScriptedBindingMatch(input_value.get("binding_matches"))
    per_user_oauth_store.credential_binding_matches = matcher
    store = LazyPerUserOAuthTokenStore(
        FixedLookup(server), store_builder=builder, redis_available=availability
    )
    steps: list[Json] = []
    for step in required(input_value.get("steps"), "steps"):
        if step["op"] == "invalidate":
            await store.invalidate(
                required(step.get("user_id"), "user_id"),
                required(step.get("server_id"), "server_id"),
            )
            steps.append({"op": "invalidate", "token": None})
        elif step["op"] == "set_redis_available":
            availability.available = required(step.get("available"), "available")
            steps.append({"op": "set_redis_available", "token": None})
        else:
            fetched = await store.fetch(
                required(step.get("user_id"), "user_id"),
                required(step.get("server_id"), "server_id"),
            )
            steps.append({"op": "fetch", "token": None if fetched is None else token_view(fetched)})
    return {
        "steps": steps,
        "builds": builder.builds,
        "store_fetches": chain.fetches,
        "store_invalidations": chain.invalidations,
        "binding_checks": matcher.calls,
    }


def run(input_value: Input) -> Json:
    if input_value["function"] == "lazy_fetch":
        return asyncio.run(run_lazy(input_value))
    return asyncio.run(run_refresh(input_value))
