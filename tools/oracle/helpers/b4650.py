from collections.abc import Callable
from typing import Protocol, TypeAlias, cast

from litellm.proxy._experimental.mcp_server.auth.token_endpoint_auth import (
    build_token_endpoint_client_auth,
    normalize_token_endpoint_auth_method,
)
from litellm.proxy._experimental.mcp_server.byok_credential_cache import (
    byok_credential_cache,
    byok_credential_cache_key,
    cache_byok_credential,
    get_cached_byok_credential,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.result import Error, Ok
from litellm.proxy._experimental.mcp_server.outbound_credentials.types import (
    ApiKeyConfig,
    AuthConfig,
    AuthorizationCodeConfig,
    AwsSigV4Config,
    ClientCredentialsConfig,
    CredError,
    IdJagConfig,
    NoneConfig,
    PassthroughConfig,
    PrivateKeyJwtAuth,
    SharedKey,
    TokenExchangeConfig,
    parse_auth_spec_kind,
)
from pydantic import SecretStr

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _ByokCache(Protocol):
    def delete_cache(self, key: str) -> None: ...


def _auth_config_variants(kind: str) -> Json:
    config = _build_auth_config(kind)
    defaults: dict[str, Json] = {}
    if isinstance(config, TokenExchangeConfig):
        defaults = {"profile": config.profile, "subject_token_type": config.subject_token_type}
    elif isinstance(config, IdJagConfig):
        client_auth = config.client_auth
        signing_alg = client_auth.signing_alg if isinstance(client_auth, PrivateKeyJwtAuth) else ""
        defaults = {
            "subject_token_type": config.subject_token_type,
            "signing_alg": signing_alg,
        }
    elif isinstance(config, AwsSigV4Config):
        defaults = {"service": config.service, "credentials": config.credentials.source}
    return {"kind": config.kind.value, "defaults": defaults}


def _byok_cache_evict(user_id: str, server_id: str, credential: str | None) -> Json:
    cache = cast(_ByokCache, byok_credential_cache)
    cache_byok_credential(user_id, server_id, credential)
    cached_before = get_cached_byok_credential(user_id, server_id) is not None
    cache.delete_cache(byok_credential_cache_key(user_id, server_id))
    cached_after = get_cached_byok_credential(user_id, server_id) is not None
    return {"cached_before": cached_before, "cached_after": cached_after}


def _byok(data: dict[str, Json]) -> Json | None:
    match data:
        case {"function": "byok_cache_key", "user_id": str(user_id), "server_id": str(server_id)}:
            return {"key": byok_credential_cache_key(user_id, server_id)}
        case {
            "function": "byok_cache_evict",
            "user_id": str(user_id),
            "server_id": str(server_id),
            "credential": credential,
        } if credential is None or isinstance(credential, str):
            return _byok_cache_evict(user_id, server_id, credential)
        case _:
            return None

_AUTH_CONFIG_BUILDERS: dict[str, Callable[[], AuthConfig]] = {
    "authorization_code": AuthorizationCodeConfig,
    "client_credentials": ClientCredentialsConfig,
    "token_exchange": TokenExchangeConfig,
    "passthrough": PassthroughConfig,
    "none": NoneConfig,
}


def _build_auth_config(kind: str) -> AuthConfig:
    builder = _AUTH_CONFIG_BUILDERS.get(kind)
    if builder is not None:
        return builder()
    if kind == "id_jag":
        return IdJagConfig(
            org_token_endpoint="https://org.example.test/token",  # noqa: S106
            resource_token_endpoint="https://resource.example.test/token",  # noqa: S106
            client_id="gateway-client",
            client_auth=PrivateKeyJwtAuth(private_key=SecretStr("private-key")),
        )
    if kind == "api_key":
        return ApiKeyConfig(key_source=SharedKey(value=SecretStr("key")))
    if kind == "aws_sigv4":
        return AwsSigV4Config(region="us-east-1")
    raise ValueError(f"unknown kind: {kind!r}")


def _result_union(ok: bool, value: str) -> Json:  # noqa: FBT001
    result = Ok(value) if ok else Error(CredError.of_misconfigured(value))
    match result:
        case Ok(ok=held):
            return {"is_ok": True, "is_error": False, "ok": held, "error_summary": None}
        case Error(error=error):
            return {"is_ok": False, "is_error": True, "ok": None, "error_summary": error.summary}


def _auth_spec_kind(raw: str) -> Json:
    match parse_auth_spec_kind(raw):
        case Ok(ok=kind):
            return {"ok": True, "value": kind.value}
        case Error(error=error):
            return {"ok": False, "error_summary": error.summary}


def _token_auth_build(
    auth_method: str | None, client_id: str | None, client_secret: str | None
) -> Json:
    method = normalize_token_endpoint_auth_method(auth_method)
    built = build_token_endpoint_client_auth(
        auth_method=method, client_id=client_id, client_secret=client_secret
    )
    return cast(Json, {"headers": built.headers, "body": built.body})


def run(data: dict[str, Json]) -> Json:
    match data:
        case {"function": "result_union", "ok": bool(ok), "value": str(value)}:
            return _result_union(ok, value)
        case {"function": "auth_spec_kind", "raw": str(raw)}:
            return _auth_spec_kind(raw)
        case {"function": "auth_config_variants", "kind": str(kind)}:
            return _auth_config_variants(kind)
        case {"function": "token_auth_normalize", "value": str(value)}:
            return {"value": normalize_token_endpoint_auth_method(value)}
        case {"function": "token_auth_build"}:
            auth_method = data.get("auth_method")
            client_id = data.get("client_id")
            client_secret = data.get("client_secret")
            if not (auth_method is None or isinstance(auth_method, str)):
                raise ValueError("auth_method must be a string or null")
            if not (client_id is None or isinstance(client_id, str)):
                raise ValueError("client_id must be a string or null")
            if not (client_secret is None or isinstance(client_secret, str)):
                raise ValueError("client_secret must be a string or null")
            return _token_auth_build(auth_method, client_id, client_secret)
        case _:
            result = _byok(data)
            if result is not None:
                return result
            raise ValueError("unknown credential_contracts function")
