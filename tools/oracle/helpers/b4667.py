import asyncio
from collections.abc import Callable, Mapping
from datetime import datetime, timezone
from typing import Final, Literal, NotRequired, TypedDict

import httpx2
from fastapi import HTTPException
from litellm.proxy._experimental.mcp_server.outbound_credentials.adapter import (
    oauth_protected_resource_path,
    raise_token_exchange_challenge,
    raise_user_oauth_challenge,
    to_server_spec,
    to_subject,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.client_credentials import (
    ClientCredentialsTokenSource,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.oauth_token_store import (
    OAuthToken,
    TokenStoreUnavailable,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.resolver import (
    UpstreamCredentialProvider,
    resolve_credentials_with_source,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.result import (
    Error,
    Ok,
    Result,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    AssertionStoreUnavailable,
    SSOIdentityAssertion,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.token_endpoint import (
    ExchangedToken,
    TokenEndpointClient,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.types import (
    Ambient,
    ApiKeyConfig,
    ApiKeySource,
    AssumeRole,
    AuthConfig,
    AuthorizationCodeConfig,
    AwsCredentialSource,
    AwsSigV4Config,
    Byok,
    ClientAuth,
    ClientCredentialsConfig,
    ClientSecretAuth,
    CredError,
    HeaderCarrier,
    IdJagConfig,
    NoneConfig,
    PassthroughConfig,
    PrivateKeyJwtAuth,
    ServerSpec,
    SharedKey,
    StaticKeys,
    Subject,
    TokenExchangeConfig,
)
from litellm.proxy._types import UserAPIKeyAuth
from litellm.types.mcp_server.mcp_server_manager import MCPServer
from pydantic import JsonValue, SecretStr, TypeAdapter

_UPSTREAM_URL: Final = "https://upstream.example.com/mcp"
_SERVER_ADAPTER: Final[TypeAdapter[MCPServer]] = TypeAdapter(MCPServer)
_JSON_ADAPTER: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
_RAISE_USER_CHALLENGE: Final[Callable[..., None]] = raise_user_oauth_challenge
_RAISE_TOKEN_EXCHANGE_CHALLENGE: Final[Callable[..., None]] = raise_token_exchange_challenge


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


class AssertionStored(TypedDict):
    kind: Literal["stored"]
    id_token: str
    expires_at_unix: int | None


class AssertionMissing(TypedDict):
    kind: Literal["missing"]


class AssertionUnavailable(TypedDict):
    kind: Literal["unavailable"]


class ExchangerToken(TypedDict):
    kind: Literal["token"]
    access_token: str


class ExchangerFailure(TypedDict):
    kind: Literal["failure"]
    tag: str
    detail: str


class CredentialsFake(TypedDict):
    access_token: str
    refetch: str | None


class CollaboratorsInput(TypedDict):
    oauth_token_store: NotRequired[TokenStoreToken | TokenStoreUnavailableInput]
    assertion_store: NotRequired[AssertionStored | AssertionMissing | AssertionUnavailable]
    token_endpoint: NotRequired[list[str]]
    exchanger: NotRequired[ExchangerToken | ExchangerFailure]
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


def json_value(value: object) -> JsonValue:
    return _JSON_ADAPTER.validate_python(value)


def server_record(record: dict[str, JsonValue]) -> MCPServer:
    return _SERVER_ADAPTER.validate_python(record)


def to_subject_answer(principal: Principal | None, subject_token: str | None) -> dict[str, JsonValue]:
    auth: UserAPIKeyAuth | None = None
    if principal is not None:
        auth = UserAPIKeyAuth(
            org_id=principal["org_id"],
            team_id=principal["team_id"],
            user_id=principal["user_id"],
        )
    subject = to_subject(auth, subject_token)
    return {
        "tenant_id": subject.tenant_id,
        "subject_id": subject.subject_id,
        "inbound_token": subject.inbound_token.get_secret_value() if subject.inbound_token is not None else None,
    }


def spec_answer(server: MCPServer) -> JsonValue:
    spec = to_server_spec(server)
    if spec is None:
        return None
    return {
        "server_id": spec.server_id,
        "resource": spec.resource,
        "config": config_answer(spec.config),
    }


def config_answer(config: AuthConfig) -> dict[str, JsonValue]:
    match config:
        case AuthorizationCodeConfig():
            answer = with_header(
                "authorization_code",
                config,
                {
                    "scopes": json_value(list(config.scopes)),
                    "client_id": config.client_id,
                    "client_secret": secret_text(config.client_secret),
                    "authorization_url": config.authorization_url,
                    "token_url": config.token_url,
                },
            )
        case ClientCredentialsConfig():
            answer = with_header(
                "client_credentials",
                config,
                {
                    "client_id": config.client_id,
                    "client_secret": secret_text(config.client_secret),
                    "token_url": config.token_url,
                    "scopes": json_value(list(config.scopes)),
                    "audience": config.audience,
                    "upstream_resource": config.upstream_resource,
                    "token_endpoint_auth_method": config.token_endpoint_auth_method,
                },
            )
        case TokenExchangeConfig():
            answer = with_header(
                "token_exchange",
                config,
                {
                    "profile": config.profile,
                    "subject_token_type": config.subject_token_type,
                    "token_exchange_endpoint": config.token_exchange_endpoint,
                    "audience": config.audience,
                    "client_id": config.client_id,
                    "client_secret": secret_text(config.client_secret),
                    "token_endpoint_auth_method": config.token_endpoint_auth_method,
                    "scopes": json_value(list(config.scopes)),
                },
            )
        case IdJagConfig():
            answer = with_header(
                "id_jag",
                config,
                {
                    "org_token_endpoint": config.org_token_endpoint,
                    "resource_token_endpoint": config.resource_token_endpoint,
                    "client_id": config.client_id,
                    "client_auth": client_auth_answer(config.client_auth),
                    "subject_token_type": config.subject_token_type,
                    "audience": config.audience,
                    "resource": config.resource,
                    "scopes": json_value(list(config.scopes)),
                },
            )
        case ApiKeyConfig():
            answer = with_header(
                "api_key",
                config,
                {"key_source": api_key_source_answer(config.key_source)},
            )
        case PassthroughConfig():
            answer = {"kind": "passthrough", "fields": {}}
        case NoneConfig():
            answer = {"kind": "none", "fields": {}}
        case AwsSigV4Config():
            answer = {
                "kind": "aws_sigv4",
                "fields": {
                    "region": config.region,
                    "service": config.service,
                    "credentials": aws_credentials_answer(config.credentials),
                },
            }
    return answer


def with_header(kind: str, carrier: HeaderCarrier, fields: dict[str, JsonValue]) -> dict[str, JsonValue]:
    return {
        "kind": kind,
        "header": {"name": carrier.header_name, "prefix": carrier.value_prefix},
        "fields": fields,
    }


def api_key_source_answer(source: ApiKeySource) -> dict[str, JsonValue]:
    match source:
        case SharedKey():
            return {"kind": "shared", "value": source.value.get_secret_value()}
        case Byok():
            return {"kind": "byok"}


def client_auth_answer(auth: ClientAuth) -> dict[str, JsonValue]:
    match auth:
        case PrivateKeyJwtAuth():
            return {
                "source": "private_key_jwt",
                "private_key": auth.private_key.get_secret_value(),
                "key_id": auth.key_id,
                "signing_alg": auth.signing_alg,
            }
        case ClientSecretAuth():
            return {"source": "client_secret", "client_secret": auth.client_secret.get_secret_value()}


def aws_credentials_answer(credentials: AwsCredentialSource) -> dict[str, JsonValue]:
    match credentials:
        case StaticKeys():
            return {
                "kind": "static_keys",
                "access_key_id": credentials.access_key_id,
                "secret_access_key": credentials.secret_access_key.get_secret_value(),
                "session_token": secret_text(credentials.session_token),
            }
        case AssumeRole():
            return {
                "kind": "assume_role",
                "role_arn": credentials.role_arn,
                "session_name": credentials.session_name,
                "external_id": credentials.external_id,
            }
        case Ambient():
            return {"kind": "ambient"}


def secret_text(value: SecretStr | None) -> str | None:
    if value is None:
        return None
    return value.get_secret_value()


def exception_answer(exception: HTTPException) -> dict[str, JsonValue]:
    headers = exception.headers if exception.headers is not None else {}
    return {
        "status_code": exception.status_code,
        "headers": json_value({name.lower(): value for name, value in headers.items()}),
    }


def user_challenge_answer(server: MCPServer, root_path: str) -> dict[str, JsonValue]:
    try:
        _RAISE_USER_CHALLENGE(server, root_path=root_path)
    except HTTPException as exception:
        return exception_answer(exception)
    user_message: Final = "the user OAuth challenge did not raise"
    raise AssertionError(user_message)


def token_exchange_challenge_answer(
    server: MCPServer, root_path: str, claims: str | None
) -> dict[str, JsonValue]:
    try:
        _RAISE_TOKEN_EXCHANGE_CHALLENGE(server, root_path=root_path, claims=claims)
    except HTTPException as exception:
        return exception_answer(exception)
    exchange_message: Final = "the token exchange challenge did not raise"
    raise AssertionError(exchange_message)


class FakeTokens:
    def __init__(self, token: OAuthToken | None, *, unavailable: bool) -> None:
        self._token = token
        self._unavailable = unavailable

    async def fetch(self, _user_id: str, _server_id: str) -> OAuthToken | None:
        if self._unavailable:
            raise TokenStoreUnavailable
        return self._token


class FakeAssertions:
    def __init__(self, assertion: SSOIdentityAssertion | None, *, unavailable: bool) -> None:
        self._assertion = assertion
        self._unavailable = unavailable

    async def fetch(self, _user_id: str) -> SSOIdentityAssertion | None:
        if self._unavailable:
            raise AssertionStoreUnavailable
        return self._assertion


class FakeEndpoint(TokenEndpointClient):
    def __init__(self, tokens: list[str]) -> None:
        super().__init__()
        self._tokens = list(tokens)
        self.calls: list[dict[str, JsonValue]] = []

    async def fetch(
        self,
        endpoint: str,
        client_id: str,
        grant_params: Mapping[str, str],
        client_auth: ClientAuth,
    ) -> Result[ExchangedToken, CredError]:
        self.calls.append(
            {
                "endpoint": endpoint,
                "client_id": client_id,
                "params": json_value(dict(grant_params)),
                "client_auth": client_auth_answer(client_auth),
            }
        )
        if not self._tokens:
            return Error(CredError.of_upstream_unavailable("no canned token endpoint reply"))
        return Ok(ExchangedToken(access_token=self._tokens.pop(0), expires_in=None))


class FakeExchanger:
    def __init__(self, token: str | None, error: CredError | None) -> None:
        self._token = token
        self._error = error
        self.exchanges: list[dict[str, JsonValue]] = []
        self.invalidations: list[dict[str, JsonValue]] = []

    async def exchange(
        self,
        subject_token: str,
        _server: ServerSpec,
        _config: TokenExchangeConfig,
        *,
        tenant_id: str = "",
    ) -> Result[OAuthToken, CredError]:
        self.exchanges.append({"subject_token": subject_token, "tenant_id": tenant_id})
        if self._error is not None:
            return Error(self._error)
        if self._token is None:
            return Error(CredError.of_misconfigured("token exchange collaborator not wired"))
        return Ok(OAuthToken(access_token=self._token))

    async def invalidate(
        self,
        subject_token: str,
        _server: ServerSpec,
        _config: TokenExchangeConfig,
        *,
        tenant_id: str = "",
    ) -> None:
        self.invalidations.append({"subject_token": subject_token, "tenant_id": tenant_id})


class FakeCredentials(ClientCredentialsTokenSource):
    def __init__(self, access_token: str | None, refetch: str | None = None) -> None:
        super().__init__()
        self._access_token = access_token
        self._refetched = refetch

    async def get(
        self, _server_id: str, _config: ClientCredentialsConfig
    ) -> Result[OAuthToken, CredError]:
        if self._access_token is None:
            return Error(CredError.of_misconfigured("client_credentials collaborator not wired"))
        return Ok(OAuthToken(access_token=self._access_token))

    async def refetch(
        self, _server_id: str, _config: ClientCredentialsConfig, _failed_access_token: str
    ) -> str | None:
        return self._refetched


def error_from_tag(tag: str, detail: str) -> CredError:
    match tag:
        case "misconfigured":
            error = CredError.of_misconfigured(detail)
        case "upstream_unavailable":
            error = CredError.of_upstream_unavailable(detail)
        case "precondition_required":
            error = CredError.of_precondition_required(detail)
        case "not_implemented":
            error = CredError.of_not_implemented(detail)
        case "unsupported_mode":
            error = CredError.of_unsupported_mode(detail)
        case "url_credentials_not_allowed":
            error = CredError.of_url_credentials_not_allowed()
        case _:
            error = CredError.of_misconfigured(detail)
    return error


def build_assertion(id_token: str, expires_at_unix: int | None) -> SSOIdentityAssertion:
    expires_at = (
        datetime.fromtimestamp(expires_at_unix, tz=timezone.utc) if expires_at_unix is not None else None
    )
    return SSOIdentityAssertion(id_token=SecretStr(id_token), expires_at=expires_at)


def build_token_store(store: TokenStoreToken | TokenStoreUnavailableInput | None) -> FakeTokens:
    if store is None:
        return FakeTokens(None, unavailable=False)
    match store["kind"]:
        case "token":
            return FakeTokens(OAuthToken(access_token=store["access_token"]), unavailable=False)
        case "unavailable":
            return FakeTokens(None, unavailable=True)


def build_assertion_store(
    store: AssertionStored | AssertionMissing | AssertionUnavailable | None,
) -> FakeAssertions:
    if store is None:
        return FakeAssertions(None, unavailable=False)
    match store["kind"]:
        case "stored":
            return FakeAssertions(
                build_assertion(store["id_token"], store["expires_at_unix"]), unavailable=False
            )
        case "missing":
            return FakeAssertions(None, unavailable=False)
        case "unavailable":
            return FakeAssertions(None, unavailable=True)


def build_exchanger(exchanger: ExchangerToken | ExchangerFailure | None) -> FakeExchanger:
    if exchanger is None:
        return FakeExchanger(None, CredError.of_misconfigured("token exchange collaborator not wired"))
    match exchanger["kind"]:
        case "token":
            return FakeExchanger(exchanger["access_token"], None)
        case "failure":
            return FakeExchanger(None, error_from_tag(exchanger["tag"], exchanger["detail"]))


def build_credentials(credentials: CredentialsFake | None) -> FakeCredentials:
    if credentials is None:
        return FakeCredentials(None)
    return FakeCredentials(credentials["access_token"], refetch=credentials["refetch"])


def build_provider(
    collaborators: CollaboratorsInput | None,
) -> tuple[UpstreamCredentialProvider, FakeEndpoint, FakeExchanger]:
    fields: CollaboratorsInput = collaborators if collaborators is not None else CollaboratorsInput()
    endpoint = FakeEndpoint(fields.get("token_endpoint") or [])
    exchanger = build_exchanger(fields.get("exchanger"))
    provider = UpstreamCredentialProvider(
        oauth_token_store=build_token_store(fields.get("oauth_token_store")),
        token_exchanger=exchanger,
        token_endpoint=endpoint,
        client_credentials_source=build_credentials(fields.get("client_credentials")),
        sso_assertion_store=build_assertion_store(fields.get("assertion_store")),
    )
    return provider, endpoint, exchanger


async def applied_headers(auth: httpx2.Auth) -> dict[str, str]:
    request = httpx2.Request("POST", _UPSTREAM_URL)
    before = dict(request.headers)
    flow = auth.async_auth_flow(request)
    try:
        _first = await anext(flow)
    finally:
        await flow.aclose()
    return {name: value for name, value in request.headers.items() if before.get(name) != value}


def error_record(action: str, error: CredError) -> dict[str, JsonValue]:
    match error.tag:
        case "unauthorized":
            return {
                "action": action,
                "ok": False,
                "tag": error.tag,
                "summary": error.summary,
                "www_authenticate": error.unauthorized.www_authenticate,
            }
        case _:
            return {
                "action": action,
                "ok": False,
                "tag": error.tag,
                "summary": error.summary,
                "www_authenticate": None,
            }


async def resolve_record(
    provider: UpstreamCredentialProvider, subject: Subject, spec: ServerSpec, action: str
) -> dict[str, JsonValue]:
    result = await provider.resolve_credentials(subject, spec)
    match result:
        case Error(error):
            return error_record(action, error)
        case Ok():
            return {
                "action": action,
                "ok": True,
                "headers": json_value(await applied_headers(result.ok)),
            }


async def source_record(
    provider: UpstreamCredentialProvider, subject: Subject, spec: ServerSpec
) -> dict[str, JsonValue]:
    result = await resolve_credentials_with_source(provider, subject, spec)
    match result:
        case Error(error):
            return error_record("resolve_with_source", error)
        case Ok():
            return {
                "action": "resolve_with_source",
                "ok": True,
                "headers": json_value(await applied_headers(result.ok.auth)),
                "source": result.ok.source.value,
            }


async def resolve_actions(
    provider: UpstreamCredentialProvider,
    subject: Subject,
    spec: ServerSpec,
    actions: list[Action],
) -> list[dict[str, JsonValue]]:
    records: list[dict[str, JsonValue]] = []
    for action in actions:
        match action:
            case "resolve":
                records.append(await resolve_record(provider, subject, spec, "resolve"))
            case "resolve_with_source":
                records.append(await source_record(provider, subject, spec))
            case "has_user_token":
                has_token = await provider.has_user_token(subject, spec)
                records.append({"action": "has_user_token", "ok": True, "has_user_token": has_token})
            case "invalidate":
                await provider.invalidate_credentials(subject, spec)
                records.append({"action": "invalidate", "ok": True})
    return records


async def resolver_answer(call: ResolverCall) -> JsonValue:
    spec = to_server_spec(server_record(call["server"]))
    if spec is None:
        return None
    provider, endpoint, exchanger = build_provider(call.get("collaborators"))
    subject = Subject(
        tenant_id=call["subject"]["tenant_id"],
        subject_id=call["subject"]["subject_id"],
        inbound_token=SecretStr(call["subject"]["inbound_token"])
        if call["subject"]["inbound_token"] is not None
        else None,
    )
    records = await resolve_actions(provider, subject, spec, call["actions"])
    return {
        "actions": json_value(records),
        "token_endpoint_calls": json_value(endpoint.calls),
        "exchanger_exchanges": json_value(exchanger.exchanges),
        "exchanger_invalidations": json_value(exchanger.invalidations),
    }


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "to_subject":
            return to_subject_answer(input_value["principal"], input_value["subject_token"])
        case "to_server_spec":
            return spec_answer(server_record(input_value["server"]))
        case "protected_resource_path":
            return oauth_protected_resource_path(
                input_value["root_path"], server_record(input_value["server"])
            )
        case "user_oauth_challenge":
            return user_challenge_answer(server_record(input_value["server"]), input_value["root_path"])
        case "token_exchange_challenge":
            return token_exchange_challenge_answer(
                server_record(input_value["server"]),
                input_value["root_path"],
                input_value["claims"],
            )
        case "credential_resolver":
            return asyncio.run(resolver_answer(input_value))
