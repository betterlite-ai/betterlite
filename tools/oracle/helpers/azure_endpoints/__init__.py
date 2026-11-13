from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.llms.azure.common_utils import BaseAzureLLM

from azure_endpoints.cloudflare import cloudflare_step
from azure_endpoints.dynamic import dynamic_step
from azure_endpoints.fixtures import identity_params, session
from azure_endpoints.inputs import (
    AuthParams,
    ClientStep,
    CloudflareStep,
    Defaults,
    InitializeStep,
    Input,
    LiteLlmParams,
    ProviderScript,
    SourceScripts,
)
from azure_endpoints.reports import ClientParams, client_report, initialize_report
from azure_endpoints.sdk import STATE, Json

ClientParamValue: TypeAlias = str | int | float | None | Callable[[], str]


def defaults_of(step: InitializeStep | ClientStep) -> Defaults | None:
    defaults: Defaults | None = None
    if "defaults" in step:
        defaults = step["defaults"]
    return defaults


def is_async_of(step: InitializeStep | ClientStep) -> bool:
    is_async = False
    if "is_async" in step:
        is_async = step["is_async"]
    return is_async


def refresh_of(step: InitializeStep | ClientStep) -> bool:
    refresh = False
    if "refresh" in step:
        refresh = step["refresh"]
    return refresh


def shared_of(step: InitializeStep | ClientStep) -> bool:
    shared = False
    if "session" in step:
        shared = step["session"] == "shared"
    return shared


def environment_of(step: InitializeStep | ClientStep | CloudflareStep) -> dict[str, str]:
    environment: dict[str, str] = {}
    if "environment" in step:
        environment = step["environment"]
    return environment


def sources_of(step: InitializeStep | ClientStep | CloudflareStep) -> SourceScripts:
    sources: SourceScripts = {}
    if "sources" in step:
        sources = step["sources"]
    return sources


def provider_of(step: InitializeStep | ClientStep | CloudflareStep) -> ProviderScript | None:
    provider: ProviderScript | None = None
    if "provider" in step:
        provider = step["provider"]
    return provider


def auth_values(auth: AuthParams) -> dict[str, ClientParamValue]:
    values: dict[str, ClientParamValue] = {}
    if "tenant_id" in auth:
        values["tenant_id"] = auth["tenant_id"]
    if "client_id" in auth:
        values["client_id"] = auth["client_id"]
    if "client_secret" in auth:
        values["client_secret"] = auth["client_secret"]
    if "azure_username" in auth:
        values["azure_username"] = auth["azure_username"]
    if "azure_password" in auth:
        values["azure_password"] = auth["azure_password"]
    if "azure_scope" in auth:
        values["azure_scope"] = auth["azure_scope"]
    return values


def token_values(params: LiteLlmParams) -> dict[str, ClientParamValue]:
    values: dict[str, ClientParamValue] = {}
    if "azure_ad_token" in params:
        values["azure_ad_token"] = params["azure_ad_token"]
    return values


def limit_values(params: LiteLlmParams) -> dict[str, ClientParamValue]:
    values: dict[str, ClientParamValue] = {}
    if "max_retries" in params:
        values["max_retries"] = params["max_retries"]
    if "timeout" in params:
        values["timeout"] = params["timeout"]
    return values


def params_values(params: LiteLlmParams) -> dict[str, ClientParamValue]:
    values = token_values(params)
    if "auth" in params:
        values.update(auth_values(params["auth"]))
    values.update(limit_values(params))
    return values


def request_params(step: InitializeStep | ClientStep) -> dict[str, ClientParamValue]:
    params: LiteLlmParams = {}
    if "params" in step:
        params = step["params"]
    values = params_values(params)
    if STATE.provider is not None:
        values["azure_ad_token_provider"] = STATE.provider
    return values


def step_api_key(step: InitializeStep | ClientStep | CloudflareStep) -> str | None:
    api_key: str | None = None
    if "api_key" in step:
        api_key = step["api_key"]
    return api_key


def step_api_base(step: InitializeStep | ClientStep) -> str | None:
    api_base: str | None = None
    if "api_base" in step:
        api_base = step["api_base"]
    return api_base


def step_api_version(step: InitializeStep | ClientStep) -> str | None:
    api_version: str | None = None
    if "api_version" in step:
        api_version = step["api_version"]
    return api_version


def initialize_step(step: InitializeStep) -> Json:
    is_async = is_async_of(step)
    with session(
        environment=environment_of(step),
        sources=sources_of(step),
        refresh=refresh_of(step),
        shared_session=shared_of(step),
        defaults=defaults_of(step),
        provider=provider_of(step),
        cached=None,
        supplied=None,
    ):
        report = run_initialize(step, is_async=is_async)
        return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L672-L696
            Json, report
        )


def run_initialize(step: InitializeStep, *, is_async: bool) -> dict[str, Json]:
    params = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L672-L696
        ClientParams,
        BaseAzureLLM().initialize_azure_sdk_client(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L602-L610
            litellm_params=request_params(step),
            api_key=step_api_key(step),
            api_base=step_api_base(step),
            model_name="gpt-4o",
            api_version=step_api_version(step),
            is_async=is_async,
        ),
    )
    return initialize_report(
        params,
        is_async=is_async,
        provider_was_passed="provider" in step,
        calls=list(STATE.calls),
    )


def client_step(step: ClientStep) -> Json:
    is_async = is_async_of(step)
    cached = "none"
    if "cached" in step:
        cached = step["cached"]
    supplied = None
    if "supplied" in step:
        supplied = step["supplied"]
    with session(
        environment=environment_of(step),
        sources=sources_of(step),
        refresh=refresh_of(step),
        shared_session=shared_of(step),
        defaults=defaults_of(step),
        provider=provider_of(step),
        cached=cached,
        supplied=supplied,
    ):
        report = run_client(step, is_async=is_async)
        return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L600
            Json, report
        )


def run_client(step: ClientStep, *, is_async: bool) -> dict[str, Json]:
    returned = BaseAzureLLM().get_azure_openai_client(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
        api_key=step_api_key(step),
        api_base=step_api_base(step),
        api_version=step_api_version(step),
        client=STATE.supplied_client,
        litellm_params=request_params(step),
        _is_async=is_async,
        model="gpt-4o",
    )
    return client_report(step, returned, is_async=is_async, identity=identity_params())


def run(argument: Input) -> Json:
    if "default_api_version" in argument:
        reports = [initialize_step(step) for step in argument["default_api_version"]]
    elif "cloudflare_base_model" in argument:
        reports = [cloudflare_step(step) for step in argument["cloudflare_base_model"]]
    elif "separate_token_client_identity" in argument:
        reports = [
            client_step(step) for step in argument["separate_token_client_identity"]
        ]
    elif "azure_sdk_client_mode" in argument:
        reports = [client_step(step) for step in argument["azure_sdk_client_mode"]]
    elif "azure_initializer_credentials" in argument:
        reports = [
            initialize_step(step) for step in argument["azure_initializer_credentials"]
        ]
    elif "azure_v1_client_projection" in argument:
        reports = [client_step(step) for step in argument["azure_v1_client_projection"]]
    elif "dynamic_api_version" in argument:
        reports = [dynamic_step(step) for step in argument["dynamic_api_version"]]
    else:
        raise ValueError("expected one azure endpoints variant")
    return {"steps": reports}
