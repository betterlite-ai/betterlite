from typing import Literal, NotRequired, TypeAlias, TypedDict


class Defaults(TypedDict):
    default_max_retries: int
    azure_default_api_version: str


class TokenScript(TypedDict):
    outcome: Literal["token"]
    token: str


class RaiseScript(TypedDict):
    outcome: Literal["raise"]
    class_name: str
    message: str


Script: TypeAlias = TokenScript | RaiseScript


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
