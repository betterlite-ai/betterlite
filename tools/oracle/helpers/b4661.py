import asyncio
from typing import Literal, NotRequired, TypedDict

import httpx2
from litellm.proxy._experimental.mcp_server.outbound_credentials.client_credentials import (
    ClientCredentialsBearerAuth,
    ClientCredentialsTokenSource,
    TokenEndpointDenied,
    TokenEndpointOutcome,
    TokenEndpointSuccess,
    TokenEndpointUnreachable,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.oauth_token_store import (
    OAuthToken,
    TokenCacheBackend,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.result import Error, Ok
from litellm.proxy._experimental.mcp_server.outbound_credentials.types import (
    ClientCredentialsConfig,
    HeaderCarrier,
)
from pydantic import JsonValue as Json
from pydantic import SecretStr
from typing_extensions import override


class ConfigSpec(TypedDict):
    client_id: NotRequired[str | None]
    client_secret: NotRequired[str | None]
    token_url: NotRequired[str | None]
    scopes: NotRequired[list[str]]
    audience: NotRequired[str | None]
    upstream_resource: NotRequired[str | None]
    token_endpoint_auth_method: NotRequired[
        Literal["client_secret_post", "client_secret_basic"] | None
    ]


class SuccessSpec(TypedDict):
    kind: Literal["success"]
    body: str


class DeniedSpec(TypedDict):
    kind: Literal["denied"]
    status_code: int
    detail: str


class UnreachableSpec(TypedDict):
    kind: Literal["unreachable"]
    detail: str


OutcomeSpec = SuccessSpec | DeniedSpec | UnreachableSpec


class GrantInput(TypedDict):
    function: Literal["grant"]
    configs: list[ConfigSpec]


class OperationSpec(TypedDict):
    server_id: str
    advance_ms: NotRequired[int]


class MintInput(TypedDict):
    function: Literal["mint"]
    config: ConfigSpec
    outcomes: list[OutcomeSpec]
    operations: list[OperationSpec]


class ScenarioSpec(TypedDict):
    access_token: str
    statuses: list[int]
    refetch: NotRequired[list[str | None]]


class BearerInput(TypedDict):
    function: Literal["bearer"]
    scenarios: list[ScenarioSpec]


Input = GrantInput | MintInput | BearerInput


class ManualClock:
    now_ms = 1_000_000

    def __call__(self) -> float:
        return self.now_ms / 1000


class CapturingBackend(TokenCacheBackend):
    def __init__(self) -> None:
        self.keys: list[str] = []

    @override
    async def get(self, user_id: str, server_id: str) -> OAuthToken | None:
        del server_id
        self.keys.append(user_id)
        return None

    @override
    async def set(
        self,
        user_id: str,
        server_id: str,
        token: OAuthToken,
        ttl_seconds: float,
    ) -> None:
        del user_id, server_id, token, ttl_seconds

    @override
    async def delete(self, user_id: str, server_id: str) -> None:
        del user_id, server_id


class ScriptedPost:
    def __init__(self, outcomes: list[TokenEndpointOutcome]) -> None:
        self.outcomes = outcomes
        self.calls: list[tuple[str, list[tuple[str, str]], list[tuple[str, str]]]] = []

    async def post(
        self,
        url: str,
        form: dict[str, str],
        headers: dict[str, str],
    ) -> TokenEndpointOutcome:
        self.calls.append((url, list(form.items()), list(headers.items())))
        return self.outcomes.pop(0)


class ScriptedRefetch:
    def __init__(self, answers: list[str | None]) -> None:
        self.answers = answers
        self.asked: list[Json] = []

    async def __call__(self, failed_access_token: str) -> str | None:
        self.asked.append(failed_access_token)
        if not self.answers:
            return None
        return self.answers.pop(0)


def build_config(spec: ConfigSpec) -> ClientCredentialsConfig:
    secret = spec.get("client_secret")
    scopes = spec.get("scopes")
    return ClientCredentialsConfig(
        client_id=spec.get("client_id"),
        client_secret=None if secret is None else SecretStr(secret),
        token_url=spec.get("token_url"),
        scopes=() if scopes is None else tuple(scopes),
        audience=spec.get("audience"),
        upstream_resource=spec.get("upstream_resource"),
        token_endpoint_auth_method=spec.get("token_endpoint_auth_method"),
    )


def build_outcome(spec: OutcomeSpec) -> TokenEndpointOutcome:
    if spec["kind"] == "success":
        return TokenEndpointSuccess.model_validate_json('{"body": ' + spec["body"] + "}")
    if spec["kind"] == "denied":
        return TokenEndpointDenied(status_code=spec["status_code"], detail=spec["detail"])
    return TokenEndpointUnreachable(detail=spec["detail"])


def token_view(token: OAuthToken) -> Json:
    expires_at = token.expires_at
    return {
        "access_token": token.access_token,
        "expires_at_ms": None if expires_at is None else round(expires_at * 1000),
        "refresh_token": token.refresh_token,
        "scopes": list(token.scopes),
        "identity_binding_proof": token.identity_binding_proof,
    }


class ScriptedTransport:
    def __init__(self, statuses: list[int]) -> None:
        self.statuses = statuses
        self.sent: list[Json] = []

    async def __call__(self, request: httpx2.Request) -> httpx2.Response:
        value = request.headers.get("Authorization")
        self.sent.append([] if value is None else [["authorization", value]])
        status = self.statuses.pop(0) if self.statuses else 0
        return httpx2.Response(status, request=request)


async def grant_view(spec: ConfigSpec) -> Json:
    config = build_config(spec)
    granted = TokenEndpointSuccess.model_validate_json('{"body": {"access_token": "oracle"}}')
    post = ScriptedPost([granted])
    backend = CapturingBackend()
    source = ClientCredentialsTokenSource(post.post, backend=backend, clock=ManualClock())
    fetch = source.get
    result = await fetch("server", config)
    match result:
        case Error(error=error):
            return {
                "error": error.summary,
                "token_url": None,
                "form": [],
                "headers": [],
                "identity_key": None,
            }
        case Ok():
            url, form, headers = post.calls[0]
            return {
                "error": None,
                "token_url": url,
                "form": [[name, value] for name, value in form],
                "headers": [[name, value] for name, value in headers],
                "identity_key": backend.keys[0],
            }


async def mint_view(input_value: MintInput) -> Json:
    config = build_config(input_value["config"])
    clock = ManualClock()
    post = ScriptedPost([build_outcome(spec) for spec in input_value["outcomes"]])
    source = ClientCredentialsTokenSource(post.post, clock=clock)
    fetch = source.get
    results: list[Json] = []
    for operation in input_value["operations"]:
        advance = operation.get("advance_ms")
        clock.now_ms += 0 if advance is None else advance
        result = await fetch(operation["server_id"], config)
        match result:
            case Ok(ok=token):
                results.append({"op": "get", "token": token_view(token)})
            case Error(error=error):
                results.append({"op": "get", "error": error.summary})
    return {
        "results": results,
        "post_calls": [
            {"url": url, "form": [[name, value] for name, value in form]}
            for url, form, _headers in post.calls
        ],
    }


async def scenario_view(scenario: ScenarioSpec) -> Json:
    answers = scenario.get("refetch")
    refetcher = ScriptedRefetch([] if answers is None else list(answers))
    auth = ClientCredentialsBearerAuth(scenario["access_token"], refetcher, HeaderCarrier())
    transport = ScriptedTransport(list(scenario["statuses"]))
    async with httpx2.AsyncClient(transport=httpx2.MockTransport(transport), auth=auth) as client:
        response = await client.post("https://upstream.example/mcp")
    view: dict[str, Json] = {}
    view["sends"] = transport.sent
    view["final_status"] = response.status_code
    view["refetched"] = refetcher.asked
    return view


def run(input_value: Input) -> Json:
    if input_value["function"] == "grant":
        views: list[Json] = [asyncio.run(grant_view(spec)) for spec in input_value["configs"]]
        return {"grants": views}
    if input_value["function"] == "mint":
        return asyncio.run(mint_view(input_value))
    scenarios: list[Json] = [
        asyncio.run(scenario_view(scenario)) for scenario in input_value["scenarios"]
    ]
    return {"scenarios": scenarios}
