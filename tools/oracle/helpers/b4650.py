from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.proxy._experimental.mcp_server.auth.token_endpoint_auth import (
    build_token_endpoint_client_auth,
    normalize_token_endpoint_auth_method,
)
from litellm.proxy._experimental.mcp_server.byok_credential_cache import byok_credential_cache_key
from litellm.proxy._experimental.mcp_server.outbound_credentials.result import Error, Ok
from litellm.proxy._experimental.mcp_server.outbound_credentials.types import (
    ApiKeyConfig,
    AuthConfig,
    AuthorizationCodeConfig,
    AwsSigV4Config,
    ClientCredentialsConfig,
    ClientSecretAuth,
    CredError,
    IdJagConfig,
    NoneConfig,
    PassthroughConfig,
    SharedKey,
    TokenExchangeConfig,
    parse_auth_spec_kind,
)
from pydantic import SecretStr

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

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
            client_auth=ClientSecretAuth(client_secret=SecretStr("secret")),
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
            return {"kind": _build_auth_config(kind).kind.value}
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
        case {"function": "byok_cache_key", "user_id": str(user_id), "server_id": str(server_id)}:
            return {"key": byok_credential_cache_key(user_id, server_id)}
        case _:
            raise ValueError("unknown credential_contracts function")
