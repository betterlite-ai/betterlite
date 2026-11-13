from typing import cast

from litellm.llms.azure.common_utils import BaseAzureLLM
from openai import AsyncAzureOpenAI, AzureOpenAI

from azure_endpoints.fixtures import session
from azure_endpoints.reports import cloudflare_report
from azure_endpoints.sdk import STATE, CloudflareStep, Json, ProviderScript, SourceScripts


def cloudflare_params(step: CloudflareStep) -> dict[str, str | None]:
    values: dict[str, str | None] = {}
    if "params" in step:
        params = step["params"]
        if "tenant_id" in params:
            values["tenant_id"] = params["tenant_id"]
        if "client_id" in params:
            values["client_id"] = params["client_id"]
        if "azure_scope" in params:
            values["azure_scope"] = params["azure_scope"]
    return values


def cloudflare_provider(step: CloudflareStep) -> str | None:
    token: str | None = None
    if "azure_ad_token" in step:
        token = step["azure_ad_token"]
    return token


def cloudflare_key(step: CloudflareStep) -> str | None:
    api_key: str | None = None
    if "api_key" in step:
        api_key = step["api_key"]
    return api_key


def cloudflare_step(step: CloudflareStep) -> Json:
    is_async = False
    if "is_async" in step:
        is_async = step["is_async"]
    shared = False
    if "shared_session" in step:
        shared = step["shared_session"]
    supplied = None
    if "client" in step:
        supplied = step["client"]
    environment: dict[str, str] = {}
    if "environment" in step:
        environment = step["environment"]
    sources: SourceScripts = {}
    if "sources" in step:
        sources = step["sources"]
    provider: ProviderScript | None = None
    if "provider" in step:
        provider = step["provider"]
    with session(
        environment=environment,
        sources=sources,
        refresh=False,
        shared_session=shared,
        defaults=None,
        provider=provider,
        cached=None,
        supplied=supplied,
    ):
        returned = BaseAzureLLM()._init_azure_client_for_cloudflare_ai_gateway(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L698-L750
            api_base=step["api_base"],
            model=step["model"],
            api_version=step["api_version"],
            max_retries=step["max_retries"],
            timeout=step["timeout"],
            litellm_params=cloudflare_params(step),
            api_key=cloudflare_key(step),
            azure_ad_token=cloudflare_provider(step),
            azure_ad_token_provider=STATE.provider,
            acompletion=is_async,
            client=cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L710
                AzureOpenAI | AsyncAzureOpenAI | None, STATE.supplied_client
            ),
        )
        report = cloudflare_report(step, returned)
        return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L698-L750
            Json, report
        )
