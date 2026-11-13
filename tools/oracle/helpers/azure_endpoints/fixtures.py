import asyncio
import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from typing import Literal, TypedDict, cast

import httpx
import litellm
from litellm.llms.azure import common_utils
from litellm.llms.openai.common_utils import BaseOpenAILLM

from azure_endpoints.inputs import (
    Defaults,
    ProviderScript,
    RaiseScript,
    Script,
    SourceScripts,
    SuppliedStep,
)
from azure_endpoints.sdk import (
    STATE,
    CachedClient,
    FixtureAsyncAzureOpenAI,
    FixtureAsyncOpenAI,
    FixtureAzureOpenAI,
    FixtureOpenAI,
    Json,
    OtherClient,
    cached_client,
    stable_id,
    supplied_client,
)

COMMON = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L626"
ACCESSORS = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/common_utils.py#L211-L259"


class IdentityParams(TypedDict, total=False):
    is_async: bool
    azure_ad_token: str | None
    azure_ad_token_provider: str


class ScriptedProvider:
    def __init__(self, script: Script) -> None:
        self.script = script

    def __call__(self) -> str:
        STATE.provider_calls += 1
        script = self.script
        if script["outcome"] == "token":
            return script["token"]
        raise exception_for(script)


def exception_for(script: RaiseScript) -> Exception:
    message = script["message"]
    match script["class_name"]:
        case "ValueError":
            return ValueError(message)
        case "KeyError":
            return KeyError(message)
        case "TypeError":
            return TypeError(message)
        case _:
            return RuntimeError(message)


def source_provider(name: str, default_token: str) -> Callable[[], str]:
    script = STATE.sources.get(name)
    if script is None:
        return ScriptedProvider({"outcome": "token", "token": default_token})
    if script["outcome"] == "raise":
        raise exception_for(script)
    return ScriptedProvider(script)


def source_call(
    name: str, default_token: str, values: dict[str, Json]
) -> Callable[[], str]:
    STATE.calls.append(values)
    return source_provider(name, default_token)


def entra_source(
    tenant_id: str, client_id: str, client_secret: str, scope: str
) -> Callable[[], str]:
    values: dict[str, Json] = {"source": "client_secret"}
    values["tenant_id"] = tenant_id
    values["client_id"] = client_id
    values["client_secret"] = client_secret
    values["scope"] = scope
    return source_call("client_secret", "client-secret-token", values)


def username_source(
    azure_username: str, azure_password: str, client_id: str, scope: str
) -> Callable[[], str]:
    values: dict[str, Json] = {
        "source": "username_password",
        "client_id": client_id,
        "azure_username": azure_username,
        "azure_password": azure_password,
        "scope": scope,
    }
    return source_call("username_password", "username-token", values)


def cached_refresh(scope: str) -> Callable[[], str]:
    STATE.calls.append({"source": "refresh", "scope": scope})
    return source_provider("refresh", "refresh-token")


def oidc_values(
    azure_ad_token: str,
    azure_client_id: str | None,
    azure_tenant_id: str | None,
    scope: str | None,
) -> dict[str, Json]:
    return {
        "source": "oidc",
        "azure_ad_token": azure_ad_token,
        "azure_client_id": azure_client_id,
        "azure_tenant_id": azure_tenant_id,
        "scope": scope,
    }


def oidc_source(
    azure_ad_token: str,
    azure_client_id: str | None,
    azure_tenant_id: str | None,
    scope: str | None,
) -> str:
    values = oidc_values(azure_ad_token, azure_client_id, azure_tenant_id, scope)
    return source_call("oidc", "oidc-token", values)()


def get_cached(
    client_initialization_params: dict[str, Json],
    client_type: Literal["openai", "azure"],
) -> CachedClient | OtherClient | None:
    _ = client_type
    STATE.identity_params = dict(client_initialization_params)
    STATE.cache_hits += 1
    return STATE.cached_client


def set_cached(
    openai_client: CachedClient,
    client_type: Literal["openai", "azure"],
    client_initialization_params: dict[str, Json],
    *,
    litellm_owned_client: bool = False,
) -> None:
    _ = (openai_client, client_type)
    STATE.identity_params = dict(client_initialization_params)
    STATE.cache_owned = litellm_owned_client


def apply_sources(sources: SourceScripts) -> None:
    STATE.sources = {}
    if "client_secret" in sources:
        STATE.sources["client_secret"] = sources["client_secret"]
    if "username_password" in sources:
        STATE.sources["username_password"] = sources["username_password"]
    if "refresh" in sources:
        STATE.sources["refresh"] = sources["refresh"]
    if "oidc" in sources:
        STATE.sources["oidc"] = sources["oidc"]


@contextmanager
def case_environment(values: dict[str, str]) -> Generator[None]:
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(values)
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(saved)


@contextmanager
def session(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L602-L696
    *,
    environment: dict[str, str],
    sources: SourceScripts,
    refresh: bool,
    shared_session: bool,
    defaults: Defaults | None,
    provider: ProviderScript | None,
    cached: str | None,
    supplied: SuppliedStep | None,
) -> Generator[None]:
    STATE.reset()
    apply_sources(sources)
    if provider is not None:
        STATE.provider = ScriptedProvider(provider["script"])
        STATE.provider_id = provider["identity"]
    if cached is not None:
        STATE.cached_client = cached_client(cached)
    if supplied is not None:
        custom_query: dict[str, str] = {}
        if "custom_query" in supplied:
            custom_query = dict(supplied["custom_query"])
        STATE.supplied_client = supplied_client(supplied["kind"], custom_query)
    STATE.constructors.clear()
    max_retries = 2
    api_version = "2025-02-01-preview"
    if defaults is not None:
        max_retries = defaults["default_max_retries"]
        api_version = defaults["azure_default_api_version"]
    sync_client = httpx.Client() if shared_session else None
    async_client = httpx.AsyncClient() if shared_session else None
    with case_environment(environment):
        litellm.enable_azure_ad_token_refresh = refresh
        litellm.client_session = sync_client
        litellm.aclient_session = async_client
        litellm.AZURE_DEFAULT_API_VERSION = api_version
        common_utils.DEFAULT_MAX_RETRIES = max_retries  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L81
        common_utils.get_azure_ad_token_from_entra_id = entra_source  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L631-L636
        common_utils.get_azure_ad_token_from_username_password = username_source  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L639-L644
        common_utils._cached_azure_ad_token_refresh_provider = cached_refresh  # type: ignore[assignment]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L99-L101
        common_utils.get_azure_ad_token_from_oidc = oidc_source  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L648-L653
        common_utils.AzureOpenAI = FixtureAzureOpenAI  # type: ignore[attr-defined, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L589-L591
        common_utils.AsyncAzureOpenAI = FixtureAsyncAzureOpenAI  # type: ignore[attr-defined, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L588-L590
        common_utils.OpenAI = FixtureOpenAI  # type: ignore[attr-defined, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L584-L586
        common_utils.AsyncOpenAI = FixtureAsyncOpenAI  # type: ignore[attr-defined, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L583-L585
        common_utils.id = stable_id  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L502
        BaseOpenAILLM.get_cached_openai_client = staticmethod(get_cached)  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/common_utils.py#L211-L221
        BaseOpenAILLM.set_cached_openai_client = staticmethod(set_cached)  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/common_utils.py#L238-L259
        try:
            yield
        finally:
            if sync_client is not None:
                sync_client.close()
            if async_client is not None:
                asyncio.run(async_client.aclose())


def identity_params() -> IdentityParams:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/common_utils.py#L261-L292
        IdentityParams, STATE.identity_params
    )
