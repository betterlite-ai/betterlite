from typing import Final, Literal, NotRequired, TypedDict

from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue, TypeAdapter

UPSTREAM_URL: Final = "https://upstream.example.com/mcp"
JSON_ADAPTER: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
SERVER_ADAPTER: Final[TypeAdapter[MCPServer]] = TypeAdapter(MCPServer)


class Principal(TypedDict):
    org_id: str | None
    team_id: str | None
    user_id: str | None


class ToSubjectCall(TypedDict):
    function: Literal["to_subject"]
    principal: Principal | None
    subject_token: str | None


class ToServerSpecCall(TypedDict):
    function: Literal["to_server_spec"]
    server: dict[str, JsonValue]


class PathCall(TypedDict):
    function: Literal["protected_resource_path"]
    server: dict[str, JsonValue]
    root_path: str
    scalar_root_path: str


class UserChallengeCall(TypedDict):
    function: Literal["user_oauth_challenge"]
    server: dict[str, JsonValue]
    root_path: str
    scalar_root_path: str


class TokenExchangeChallengeCall(TypedDict):
    function: Literal["token_exchange_challenge"]
    server: dict[str, JsonValue]
    root_path: str
    scalar_root_path: str
    claims: str | None


class SubjectInput(TypedDict):
    tenant_id: str
    subject_id: str
    inbound_token: str | None


class TokenStoreToken(TypedDict):
    kind: Literal["token"]
    access_token: str


class TokenStoreUnavailableInput(TypedDict):
    kind: Literal["unavailable"]


TokenStoreInput = TokenStoreToken | TokenStoreUnavailableInput


class AssertionStored(TypedDict):
    kind: Literal["stored"]
    id_token: str
    expires_at_unix: int | None


class AssertionMissing(TypedDict):
    kind: Literal["missing"]


class AssertionUnavailable(TypedDict):
    kind: Literal["unavailable"]


AssertionInput = AssertionStored | AssertionMissing | AssertionUnavailable


class ExchangerToken(TypedDict):
    kind: Literal["token"]
    access_token: str


class ExchangerFailure(TypedDict):
    kind: Literal["failure"]
    tag: str
    detail: str


ExchangerInput = ExchangerToken | ExchangerFailure


class CredentialsFake(TypedDict):
    access_token: str
    refetch: str | None


class CollaboratorsInput(TypedDict):
    oauth_token_store: NotRequired[TokenStoreInput]
    assertion_store: NotRequired[AssertionInput]
    token_endpoint: NotRequired[list[str]]
    exchanger: NotRequired[ExchangerInput]
    client_credentials: NotRequired[CredentialsFake]


Action = Literal["resolve", "resolve_with_source", "has_user_token", "invalidate"]


class ResolverCall(TypedDict):
    function: Literal["credential_resolver"]
    server: dict[str, JsonValue]
    subject: SubjectInput
    collaborators: NotRequired[CollaboratorsInput]
    actions: list[Action]


Input = (
    ToSubjectCall
    | ToServerSpecCall
    | PathCall
    | UserChallengeCall
    | TokenExchangeChallengeCall
    | ResolverCall
)
