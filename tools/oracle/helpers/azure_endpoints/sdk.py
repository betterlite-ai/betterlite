import builtins
import inspect
from collections.abc import Callable
from typing import Literal, NotRequired, TypeAlias, TypedDict, Unpack

import httpx
import litellm
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue as Json

CachedClient: TypeAlias = OpenAI | AsyncOpenAI | AzureOpenAI | AsyncAzureOpenAI


class TokenScript(TypedDict):
    outcome: Literal["token"]
    token: str


class RaiseScript(TypedDict):
    outcome: Literal["raise"]
    class_name: str
    message: str


Script: TypeAlias = TokenScript | RaiseScript


class Defaults(TypedDict):
    default_max_retries: int
    azure_default_api_version: str


class ProviderScript(TypedDict):
    identity: int
    script: Script


class SourceScripts(TypedDict, total=False):
    client_secret: Script
    username_password: Script
    refresh: Script
    oidc: Script


class AuthParams(TypedDict, total=False):
    tenant_id: str | None
    client_id: str | None
    client_secret: str | None
    azure_username: str | None
    azure_password: str | None
    azure_scope: str | None


class LiteLlmParams(TypedDict, total=False):
    azure_ad_token: str | None
    auth: AuthParams
    max_retries: int | None
    timeout: float | None


class InitializeStep(TypedDict, total=False):
    defaults: Defaults
    params: LiteLlmParams
    api_key: str
    api_base: str
    api_version: str
    is_async: bool
    session: Literal["dedicated", "shared"]
    refresh: bool
    provider: ProviderScript
    environment: dict[str, str]
    sources: SourceScripts


class SuppliedStep(TypedDict):
    kind: Literal["azure_open_ai", "async_azure_open_ai", "open_ai", "async_open_ai"]
    custom_query: NotRequired[dict[str, str]]


class ClientStep(InitializeStep, total=False):
    cached: Literal[
        "none", "azure_open_ai", "async_azure_open_ai", "open_ai", "async_open_ai", "other"
    ]
    supplied: SuppliedStep


class CloudflareParams(TypedDict, total=False):
    tenant_id: str | None
    client_id: str | None
    azure_scope: str | None


class CloudflareStep(TypedDict):
    api_base: str
    model: str
    api_version: str
    max_retries: int
    timeout: float
    params: NotRequired[CloudflareParams]
    api_key: NotRequired[str]
    azure_ad_token: NotRequired[str]
    provider: NotRequired[ProviderScript]
    shared_session: NotRequired[bool]
    is_async: NotRequired[bool]
    client: NotRequired[SuppliedStep]
    environment: NotRequired[dict[str, str]]
    sources: NotRequired[SourceScripts]


class DynamicStep(TypedDict, total=False):
    api_version: str
    custom_query: dict[str, str]


class Input(TypedDict, total=False):
    default_api_version: list[InitializeStep]
    cloudflare_base_model: list[CloudflareStep]
    separate_token_client_identity: list[ClientStep]
    azure_sdk_client_mode: list[ClientStep]
    azure_initializer_credentials: list[InitializeStep]
    azure_v1_client_projection: list[ClientStep]
    dynamic_api_version: list[DynamicStep]


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


class IdentityParams(TypedDict, total=False):
    is_async: bool
    azure_ad_token: str | None
    azure_ad_token_provider: str


class Capture:
    def __init__(self) -> None:
        self.calls: list[dict[str, Json]] = []
        self.constructors: list[dict[str, Json]] = []
        self.identity_params: IdentityParams = {}
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
    api_key = init.get("api_key")
    return {
        "api_key_kind": key_kind(api_key),
        "api_key": api_key_value(api_key),
        "azure_ad_token": init.get("azure_ad_token"),
        "token_provider": init.get("azure_ad_token_provider") is not None,
        "timeout": init.get("timeout"),
        "max_retries": init.get("max_retries"),
        "http_shared": is_shared_http(init.get("http_client")),
        "azure_endpoint": init.get("azure_endpoint"),
        "base_url": init.get("base_url"),
        "api_version": init.get("api_version"),
    }


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


def stable_id(value: Callable[[], str]) -> int:
    if value is STATE.provider:
        return STATE.provider_id
    return builtins.id(value)
