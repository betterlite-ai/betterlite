import os
import sys
import types
from typing import TypedDict
from unittest import mock
from urllib.parse import parse_qsl

from botocore.credentials import Credentials
from litellm._redis import (
    _build_azure_credential,  # pyright: ignore[reportPrivateUsage]
)
from litellm._redis_credential_provider import (
    _GCP_IAM_TOKEN_TTL_SECONDS,  # pyright: ignore[reportPrivateUsage]
    ElastiCacheIAMCredentialProvider,
    GCPIAMCredentialProvider,
)

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_STATIC_AWS_CREDENTIALS = Credentials(
    access_key="AKIAIOSFODNN7EXAMPLE",
    secret_key="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",  # noqa: S106
)

AZURE_ENVIRONMENT_NAMES = ("AZURE_CLIENT_ID", "AZURE_TENANT_ID", "AZURE_CLIENT_SECRET")
AZURE_CREDENTIAL_NAMES = {
    "ClientSecretCredential": "client_secret",
    "ManagedIdentityCredential": "managed_identity",
    "DefaultAzureCredential": "default",
}


class Input(TypedDict):
    function: str
    service_account: str
    cache_name: str
    serverless: bool
    client_id: str | None
    tenant_id: str | None
    client_secret: str | None


def gcp_token_caching(service_account: str) -> Json:
    if not service_account:
        return {"provider": "none", "service_account_configured": False, "token_ttl_seconds": None}
    provider = GCPIAMCredentialProvider(service_account)
    configured = bool(provider._gcp_service_account)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {
        "provider": "gcp",
        "service_account_configured": configured,
        "token_ttl_seconds": _GCP_IAM_TOKEN_TTL_SECONDS,
    }


def elasticache_sig_generation(cache_name: str, *, serverless: bool) -> Json:
    provider = ElastiCacheIAMCredentialProvider(
        user_name="connect",
        cache_name=cache_name,
        region="us-east-1",
        is_serverless=serverless,
        credentials_resolver=lambda: _STATIC_AWS_CREDENTIALS,
    )
    _user_name, signed_url = provider.get_credentials()
    host, _, query = signed_url.partition("/?")
    params = dict(parse_qsl(query))
    result: dict[str, Json] = {
        "provider": "aws",
        "host": host,
        "User": params.get("User"),
        "Action": params.get("Action"),
    }
    if "ResourceType" in params:
        result["ResourceType"] = params["ResourceType"]
    return result


class _StubAzureCredential:
    def __init__(self, **_kwargs: object) -> None:
        pass


class _StubAzureIdentityModule(types.ModuleType):
    DefaultAzureCredential: type[_StubAzureCredential]
    ManagedIdentityCredential: type[_StubAzureCredential]
    ClientSecretCredential: type[_StubAzureCredential]

    def __init__(self) -> None:
        super().__init__("azure.identity")
        self.DefaultAzureCredential = type("DefaultAzureCredential", (_StubAzureCredential,), {})
        self.ManagedIdentityCredential = type(
            "ManagedIdentityCredential", (_StubAzureCredential,), {}
        )
        self.ClientSecretCredential = type("ClientSecretCredential", (_StubAzureCredential,), {})


def azure_credential_selection(
    client_id: str | None, tenant_id: str | None, client_secret: str | None
) -> Json:
    stub = _StubAzureIdentityModule()
    with (
        mock.patch.dict(sys.modules, {"azure.identity": stub}),
        mock.patch.dict(os.environ, {}, clear=False),
    ):
        for name in AZURE_ENVIRONMENT_NAMES:
            os.environ.pop(name, None)
        credential = _build_azure_credential(
            azure_client_id=client_id,
            azure_tenant_id=tenant_id,
            azure_client_secret=client_secret,
        )
    return {"provider": "azure", "credential": AZURE_CREDENTIAL_NAMES[type(credential).__name__]}


def run(data: Input) -> Json:
    match data["function"]:
        case "gcp_token_caching":
            return gcp_token_caching(data["service_account"])
        case "elasticache_sig_generation":
            serverless = data.get("serverless", False)
            return elasticache_sig_generation(data["cache_name"], serverless=serverless)
        case "azure_credential_selection":
            return azure_credential_selection(
                data.get("client_id"), data.get("tenant_id"), data.get("client_secret")
            )
        case _:
            raise ValueError("unknown redis credentials function")
