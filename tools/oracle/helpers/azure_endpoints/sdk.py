import builtins
import inspect
from collections.abc import Callable
from typing import TypeAlias, TypedDict, Unpack

import httpx
import litellm
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI

from azure_endpoints.inputs import Script

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
CachedClient: TypeAlias = OpenAI | AsyncOpenAI | AzureOpenAI | AsyncAzureOpenAI

SOURCE = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L589"


class AzureInit(TypedDict, total=False):
    azure_endpoint: str | None
    base_url: str | None
    api_version: str | None
    api_key: str | Callable[[], str] | None
    azure_ad_token: str | None
    azure_ad_token_provider: Callable[[], str] | None
    timeout: float
    max_retries: int
    http_client: httpx.Client | httpx.AsyncClient | None


class OpenAiInit(TypedDict, total=False):
    api_key: str | Callable[[], str] | None
    base_url: str | None
    timeout: float
    max_retries: int
    http_client: httpx.Client | httpx.AsyncClient | None


class Capture:
    def __init__(self) -> None:
        self.calls: list[dict[str, Json]] = []
        self.constructors: list[dict[str, Json]] = []
        self.identity_params: dict[str, Json] = {}
        self.cache_owned: bool | None = None
        self.cache_hits = 0
        self.provider_calls = 0
        self.provider: Callable[[], str] | None = None
        self.provider_id = 0
        self.cached_client: CachedClient | OtherClient | None = None
        self.supplied_client: CachedClient | None = None
        self.sources: dict[str, Script] = {}

    def reset(self) -> None:
        self.calls.clear()
        self.constructors.clear()
        self.identity_params = {}
        self.cache_owned = None
        self.cache_hits = 0
        self.provider_calls = 0
        self.provider = None
        self.provider_id = 0
        self.cached_client = None
        self.supplied_client = None
        self.sources = {}


class OtherClient:
    def __init__(self) -> None:
        self.marker = "other"


class QueryClient:
    def __init__(self, custom_query: dict[str, str]) -> None:
        self._custom_query = custom_query


STATE = Capture()


def is_shared_http(http_client: httpx.Client | httpx.AsyncClient | None) -> bool:
    if http_client is None:
        return False
    return http_client is litellm.client_session or http_client is litellm.aclient_session


def key_kind(api_key: str | Callable[[], str] | None) -> str:
    if api_key is None:
        return "none"
    if callable(api_key):
        return "provider"
    return "static"


def api_key_value(api_key: str | Callable[[], str] | None) -> str | None:
    if api_key is None or callable(api_key):
        return None
    return api_key


def provider_is_async(api_key: str | Callable[[], str] | None) -> bool:
    if not callable(api_key):
        return False
    return inspect.iscoroutinefunction(api_key)


def record_constructor(kind: str, values: dict[str, Json]) -> None:
    values["kind"] = kind
    STATE.constructors.append(values)


def azure_values(init: AzureInit) -> dict[str, Json]:
    values: dict[str, Json] = {
        "api_key_kind": key_kind(init.get("api_key")),
        "api_key": api_key_value(init.get("api_key")),
        "azure_ad_token": init.get("azure_ad_token"),
        "token_provider": init.get("azure_ad_token_provider") is not None,
        "timeout": init.get("timeout"),
        "max_retries": init.get("max_retries"),
        "http_shared": is_shared_http(init.get("http_client")),
        "azure_endpoint": init.get("azure_endpoint"),
        "base_url": init.get("base_url"),
        "api_version": init.get("api_version"),
    }
    return values


def openai_values(init: OpenAiInit) -> dict[str, Json]:
    api_key = init.get("api_key")
    values: dict[str, Json] = {
        "api_key_kind": key_kind(api_key),
        "api_key_provider": provider_is_async(api_key),
    }
    values["base_url"] = init.get("base_url")
    values["timeout"] = init.get("timeout")
    values["max_retries"] = init.get("max_retries")
    values["http_shared"] = is_shared_http(init.get("http_client"))
    return values


class FixtureAzureOpenAI(AzureOpenAI):  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L589
    custom_query: dict[str, str]

    def __init__(self, **kwargs: Unpack[AzureInit]) -> None:
        self.custom_query = {}
        self._custom_query = self.custom_query
        record_constructor("azure", azure_values(kwargs))

    @classmethod
    def blank(cls, custom_query: dict[str, str]) -> "FixtureAzureOpenAI":
        client = object.__new__(cls)
        client.custom_query = dict(custom_query)
        client._custom_query = client.custom_query  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L517
        return client


class FixtureAsyncAzureOpenAI(AsyncAzureOpenAI):  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L589
    custom_query: dict[str, str]

    def __init__(self, **kwargs: Unpack[AzureInit]) -> None:
        self.custom_query = {}
        self._custom_query = self.custom_query
        record_constructor("async_azure", azure_values(kwargs))

    @classmethod
    def blank(cls, custom_query: dict[str, str]) -> "FixtureAsyncAzureOpenAI":
        client = object.__new__(cls)
        client.custom_query = dict(custom_query)
        client._custom_query = client.custom_query  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L517
        return client


class FixtureOpenAI(OpenAI):  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L583
    def __init__(self, **kwargs: Unpack[OpenAiInit]) -> None:
        record_constructor("openai", openai_values(kwargs))

    @classmethod
    def blank(cls) -> "FixtureOpenAI":
        return object.__new__(cls)


class FixtureAsyncOpenAI(AsyncOpenAI):  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L583
    def __init__(self, **kwargs: Unpack[OpenAiInit]) -> None:
        record_constructor("async_openai", openai_values(kwargs))

    @classmethod
    def blank(cls) -> "FixtureAsyncOpenAI":
        return object.__new__(cls)


def cached_client(kind: str) -> CachedClient | OtherClient | None:
    match kind:
        case "none":
            return None
        case "other":
            return OtherClient()
        case "azure_open_ai":
            return FixtureAzureOpenAI.blank({})
        case "async_azure_open_ai":
            return FixtureAsyncAzureOpenAI.blank({})
        case "open_ai":
            return FixtureOpenAI.blank()
        case "async_open_ai":
            return FixtureAsyncOpenAI.blank()
        case _:
            raise ValueError(f"unknown cached client {kind}")


def supplied_client(kind: str, custom_query: dict[str, str]) -> CachedClient:
    match kind:
        case "azure_open_ai":
            return FixtureAzureOpenAI.blank(custom_query)
        case "async_azure_open_ai":
            return FixtureAsyncAzureOpenAI.blank(custom_query)
        case "open_ai":
            return FixtureOpenAI.blank()
        case "async_open_ai":
            return FixtureAsyncOpenAI.blank()
        case _:
            raise ValueError(f"unknown supplied client {kind}")


def stable_id(value: Callable[[], str]) -> int:
    if value is STATE.provider:
        return STATE.provider_id
    return builtins.id(value)
