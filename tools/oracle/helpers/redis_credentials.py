import os
import sys
import types
from typing import TypedDict
from unittest import mock
from urllib.parse import urlencode

from litellm._redis import (
    _build_azure_credential,  # pyright: ignore[reportPrivateUsage]
    _build_elasticache_iam_provider,  # pyright: ignore[reportPrivateUsage]
)
from litellm._redis_credential_provider import (
    _ELASTICACHE_SERVERLESS_RESOURCE_TYPE,  # pyright: ignore[reportPrivateUsage]
    _ELASTICACHE_TOKEN_TTL_SECONDS,  # pyright: ignore[reportPrivateUsage]
    _GCP_IAM_TOKEN_TTL_SECONDS,  # pyright: ignore[reportPrivateUsage]
    GCPIAMCredentialProvider,
)

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

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
    provider = _build_elasticache_iam_provider(
        {
            "aws_iam_user_name": "connect",
            "aws_iam_cache_name": cache_name,
            "aws_iam_region": "us-east-1",
            "aws_iam_serverless": serverless,
        }
    )
    user_name = provider._user_name  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    is_serverless = provider._is_serverless  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    cache_name_resolved = provider._cache_name  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    query = urlencode(
        (
            ("Action", "connect"),
            ("User", user_name),
            *((("ResourceType", _ELASTICACHE_SERVERLESS_RESOURCE_TYPE),) if is_serverless else ()),
        )
    )
    return {
        "provider": "aws",
        "cache_name": cache_name_resolved,
        "token_ttl_seconds": _ELASTICACHE_TOKEN_TTL_SECONDS,
        "query": query,
    }


class _StubAzureCredential:
    def __init__(self, **_kwargs: object) -> None:
        pass


def _stub_azure_identity_module() -> types.ModuleType:
    stub = types.ModuleType("azure.identity")
    stub.DefaultAzureCredential = type("DefaultAzureCredential", (_StubAzureCredential,), {})  # type: ignore[attr-defined]
    stub.ManagedIdentityCredential = type("ManagedIdentityCredential", (_StubAzureCredential,), {})  # type: ignore[attr-defined]
    stub.ClientSecretCredential = type("ClientSecretCredential", (_StubAzureCredential,), {})  # type: ignore[attr-defined]
    return stub


def azure_credential_selection(
    client_id: str | None, tenant_id: str | None, client_secret: str | None
) -> Json:
    stub = _stub_azure_identity_module()
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
