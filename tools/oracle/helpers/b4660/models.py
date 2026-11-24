from typing import Literal, NotRequired, TypedDict, TypeVar

from litellm.proxy._experimental.mcp_server.oauth_identity_binding import (
    RefreshOwnership,
    RefreshOwnershipProven,
    RefreshTokenPresented,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.oauth_token_store import (
    OAuthToken,
)
from pydantic import JsonValue as Json

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
