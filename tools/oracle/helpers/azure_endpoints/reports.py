from collections.abc import Callable
from typing import NotRequired, TypedDict, cast

import httpx
import litellm
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue as Json

from azure_endpoints.fixtures import IdentityParams
from azure_endpoints.sdk import (
    STATE,
    CachedClient,
    ClientStep,
    CloudflareStep,
    FixtureAsyncAzureOpenAI,
    FixtureAzureOpenAI,
    SuppliedStep,
)


def calls_json(calls: list[dict[str, Json]]) -> Json:
    value: Json = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L534
        Json, calls
    )
    return value


class ClientParams(TypedDict):
    api_version: str
    max_retries: int
    api_key: NotRequired[str | None]
    azure_endpoint: NotRequired[str | None]
    base_url: NotRequired[str | None]
    azure_ad_token: NotRequired[str | None]
    azure_ad_token_provider: NotRequired[Callable[[], str] | None]
    http_client: NotRequired[httpx.Client | httpx.AsyncClient | None]
    timeout: NotRequired[float]


def kind_name(client: CachedClient) -> str:
    match client:
        case AsyncAzureOpenAI():
            return "async_azure"
        case AzureOpenAI():
            return "azure"
        case AsyncOpenAI():
            return "async_openai"
        case OpenAI():
            return "openai"
    raise ValueError("unknown client family")


def call_source(call: dict[str, Json]) -> str:
    source = call.get("source")
    if source == "client_secret":
        return "client_secret"
    if source == "username_password":
        return "username_password"
    if source == "refresh":
        return "refresh"
    return "none"


def provider_kind(*, provider_was_passed: bool, calls: list[dict[str, Json]]) -> str:
    if provider_was_passed:
        return "passed"
    found = "none"
    for call in calls:
        if found == "none":
            found = call_source(call)
    return found


def identity_json(identity: IdentityParams) -> dict[str, Json]:
    is_async = False
    if "is_async" in identity:
        is_async = identity["is_async"]
    ad_token: str | None = None
    if "azure_ad_token" in identity:
        ad_token = identity["azure_ad_token"]
    provider = ""
    if "azure_ad_token_provider" in identity:
        provider = identity["azure_ad_token_provider"]
    report: dict[str, Json] = {
        "is_async": is_async,
        "azure_ad_token": ad_token,
        "azure_ad_token_provider": provider,
    }
    return report


def initialize_report(
    params: ClientParams,
    *,
    is_async: bool,
    provider_was_passed: bool,
    calls: list[dict[str, Json]],
) -> dict[str, Json]:
    http_client: httpx.Client | httpx.AsyncClient | None = None
    if "http_client" in params:
        http_client = params["http_client"]
    azure_endpoint: str | None = None
    if "azure_endpoint" in params:
        azure_endpoint = params["azure_endpoint"]
    base_url: str | None = None
    if "base_url" in params:
        base_url = params["base_url"]
    timeout: float | None = None
    if "timeout" in params:
        timeout = params["timeout"]
    api_key: str | None = None
    if "api_key" in params:
        api_key = params["api_key"]
    ad_token: str | None = None
    if "azure_ad_token" in params:
        ad_token = params["azure_ad_token"]
    endpoint: dict[str, Json] = {
        "azure_endpoint": azure_endpoint,
        "base_url": base_url,
    }
    report: dict[str, Json] = {
        "api_version": params["api_version"],
        "max_retries": params["max_retries"],
        "timeout": timeout,
        "http_client": {
            "is_async": is_async,
            "shared": http_client is litellm.client_session
            or http_client is litellm.aclient_session,
        },
        "endpoint": endpoint,
        "api_key": api_key,
        "azure_ad_token": ad_token,
        "provider": provider_kind(
            provider_was_passed=provider_was_passed, calls=calls
        ),
        "calls": calls_json(calls),
    }
    return report


def custom_query_of(step: SuppliedStep, client: CachedClient | None) -> Json:
    if client is None:
        return None
    if step["kind"] in ("open_ai", "async_open_ai"):
        return None
    azure_client = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L517
        FixtureAzureOpenAI | FixtureAsyncAzureOpenAI, client
    )
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L517
        Json, dict(azure_client.custom_query)
    )


def client_source(step: ClientStep, returned: CachedClient | None) -> str:
    if "supplied" in step:
        return "supplied"
    if STATE.cache_hits > 0 and returned is STATE.cached_client:
        return "cached"
    return "constructed"


def client_report(
    step: ClientStep,
    returned: CachedClient | None,
    *,
    is_async: bool,
    identity: IdentityParams,
) -> dict[str, Json]:
    source = client_source(step, returned)
    kind = "other"
    if returned is not None:
        kind = kind_name(returned)
    query: Json = None
    if "supplied" in step:
        query = custom_query_of(step["supplied"], returned)
    report: dict[str, Json] = {
        "source": source,
        "kind": kind,
        "custom_query": query,
        "cache_owned": STATE.cache_owned,
        "identity": identity_json(identity),
        "inputs": inputs_json(source=source, returned=returned, is_async=is_async),
        "calls": calls_json(list(STATE.calls)),
    }
    return report


def inputs_json(*, source: str, returned: CachedClient | None, is_async: bool) -> Json:
    if source != "constructed" or returned is None or not STATE.constructors:
        return None
    record = STATE.constructors[-1]
    kind = kind_name(returned)
    if kind in ("openai", "async_openai"):
        return {"open_ai_v1": v1_inputs(record, is_async=is_async)}
    return {"azure": azure_inputs(record, is_async=is_async)}


def v1_inputs(record: dict[str, Json], *, is_async: bool) -> dict[str, Json]:
    key: dict[str, Json] = {
        "kind": record["api_key_kind"],
        "offload": record["api_key_provider"],
    }
    http: dict[str, Json] = {"is_async": is_async, "shared": record["http_shared"]}
    values: dict[str, Json] = {"key": key, "base_url": record["base_url"]}
    values["timeout"] = record["timeout"]
    values["max_retries"] = record["max_retries"]
    values["http_client"] = http
    return values


def azure_inputs(record: dict[str, Json], *, is_async: bool) -> dict[str, Json]:
    endpoint: dict[str, Json] = {
        "azure_endpoint": record["azure_endpoint"],
        "base_url": record["base_url"],
    }
    values: dict[str, Json] = {"endpoint": endpoint}
    values["api_version"] = record["api_version"]
    values["api_key"] = record["api_key"]
    values["azure_ad_token"] = record["azure_ad_token"]
    values["token_provider"] = record["token_provider"]
    values["max_retries"] = record["max_retries"]
    values["timeout"] = record["timeout"]
    values["http_client"] = {"is_async": is_async, "shared": record["http_shared"]}
    return values


def cloudflare_report(
    step: CloudflareStep,
    returned: AzureOpenAI | AsyncAzureOpenAI,
) -> dict[str, Json]:
    calls = list(STATE.calls)
    if "client" in step:
        report: dict[str, Json] = {
            "kind": "supplied",
            "supplied": supplied_json(step["client"], returned),
            "client": None,
            "calls": calls_json(calls),
        }
        return report
    record = STATE.constructors[-1]
    return {
        "kind": "constructed",
        "supplied": None,
        "client": constructed_json(record),
        "calls": calls_json(calls),
    }


def supplied_json(step: SuppliedStep, returned: CachedClient) -> dict[str, Json]:
    query: Json = None
    if "custom_query" in step:
        query = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L731
            Json, dict(step["custom_query"])
        )
    report: dict[str, Json] = {
        "kind": kind_name(returned),
        "custom_query": query,
    }
    return report


def constructed_json(record: dict[str, Json]) -> dict[str, Json]:
    report: dict[str, Json] = {
        "base_url": record["base_url"],
        "api_version": record["api_version"],
        "api_key": record["api_key"],
        "azure_ad_token": record["azure_ad_token"],
        "token_provider": record["token_provider"],
        "max_retries": record["max_retries"],
        "timeout": record["timeout"],
        "shared_session": record["http_shared"],
        "kind": record["kind"],
    }
    return report
