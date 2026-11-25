from typing import assert_never

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
    HeaderCarrier,
    IdJagConfig,
    NoneConfig,
    PassthroughConfig,
    PrivateKeyJwtAuth,
    SharedKey,
    StaticKeys,
    TokenExchangeConfig,
)
from pydantic import JsonValue, SecretStr

from b4667.wire import JSON_ADAPTER

GrantConfig = (
    AuthorizationCodeConfig
    | ClientCredentialsConfig
    | TokenExchangeConfig
    | IdJagConfig
)


def config_scopes(config: GrantConfig) -> list[str]:
    scopes: list[str] = list(config.scopes)
    return scopes


def config_answer(config: AuthConfig) -> dict[str, JsonValue]:
    match config:
        case (
            AuthorizationCodeConfig()
            | ClientCredentialsConfig()
            | TokenExchangeConfig()
            | IdJagConfig()
        ):
            return grant_config_answer(config)
        case ApiKeyConfig():
            key_source = api_key_source_answer(config.key_source)
            return with_header("api_key", config, {"key_source": key_source})
        case PassthroughConfig():
            passthrough_fields: dict[str, JsonValue] = {}
            return {"kind": "passthrough", "fields": passthrough_fields}
        case NoneConfig():
            none_fields: dict[str, JsonValue] = {}
            return {"kind": "none", "fields": none_fields}
        case AwsSigV4Config():
            return aws_config_answer(config)
    assert_never(config)


def grant_config_answer(config: GrantConfig) -> dict[str, JsonValue]:
    match config:
        case AuthorizationCodeConfig():
            return with_header(
                "authorization_code",
                config,
                {
                    "scopes": JSON_ADAPTER.validate_python(config_scopes(config)),
                    "client_id": config.client_id,
                    "client_secret": secret_text(config.client_secret),
                    "authorization_url": config.authorization_url,
                    "token_url": config.token_url,
                },
            )
        case ClientCredentialsConfig():
            return with_header(
                "client_credentials",
                config,
                {
                    "client_id": config.client_id,
                    "client_secret": secret_text(config.client_secret),
                    "token_url": config.token_url,
                    "scopes": JSON_ADAPTER.validate_python(config_scopes(config)),
                    "audience": config.audience,
                    "upstream_resource": config.upstream_resource,
                    "token_endpoint_auth_method": config.token_endpoint_auth_method,
                },
            )
        case TokenExchangeConfig():
            return with_header(
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
                    "scopes": JSON_ADAPTER.validate_python(config_scopes(config)),
                },
            )
        case IdJagConfig():
            return with_header(
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
                    "scopes": JSON_ADAPTER.validate_python(config_scopes(config)),
                },
            )
    assert_never(config)


def aws_config_answer(config: AwsSigV4Config) -> dict[str, JsonValue]:
    return {
        "kind": "aws_sigv4",
        "fields": {
            "region": config.region,
            "service": config.service,
            "credentials": aws_credentials_answer(config.credentials),
        },
    }


def with_header(
    kind: str, carrier: HeaderCarrier, fields: dict[str, JsonValue]
) -> dict[str, JsonValue]:
    header: dict[str, JsonValue] = {
        "name": carrier.header_name,
        "prefix": carrier.value_prefix,
    }
    return {"kind": kind, "header": header, "fields": fields}


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
            return {
                "source": "client_secret",
                "client_secret": auth.client_secret.get_secret_value(),
            }


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
