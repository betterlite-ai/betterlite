import httpx
from openai import AsyncAzureOpenAI, AzureOpenAI

from .fixture import Fixture
from .types import API_BASE, API_VERSION, CallCase


def source_key(case: CallCase) -> str | None:
    if "client_api_key" in case:
        return case["client_api_key"]
    return case.get("api_key")


def sync_client(case: CallCase, fixture: Fixture) -> AzureOpenAI:
    key = source_key(case)
    token = None if key is not None else case.get("azure_ad_token")
    version = case.get("api_version")
    endpoint = case.get("api_base")
    return AzureOpenAI(
        api_key=key,
        azure_ad_token=token,
        api_version=API_VERSION if version is None else version,
        azure_endpoint=API_BASE if endpoint is None else endpoint,
        max_retries=0,
        http_client=httpx.Client(transport=httpx.MockTransport(fixture)),
    )


def async_client(case: CallCase, fixture: Fixture) -> AsyncAzureOpenAI:
    key = source_key(case)
    token = None if key is not None else case.get("azure_ad_token")
    version = case.get("api_version")
    endpoint = case.get("api_base")
    return AsyncAzureOpenAI(
        api_key=key,
        azure_ad_token=token,
        api_version=API_VERSION if version is None else version,
        azure_endpoint=API_BASE if endpoint is None else endpoint,
        max_retries=0,
        http_client=httpx.AsyncClient(transport=httpx.MockTransport(fixture)),
    )


def client_for(case: CallCase, fixture: Fixture) -> AzureOpenAI | AsyncAzureOpenAI:
    requested = case.get("client")
    if "client" not in case:
        requested = "async" if case.get("acompletion") else "sync"
    if requested == "async":
        return async_client(case, fixture)
    return sync_client(case, fixture)
