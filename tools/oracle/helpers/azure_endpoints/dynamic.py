from typing import cast

from litellm.llms.azure import azure as azure_module
from openai import AsyncAzureOpenAI, AzureOpenAI

from azure_endpoints.inputs import DynamicStep
from azure_endpoints.sdk import Json, QueryClient


def dynamic_step(step: DynamicStep) -> Json:
    params: dict[str, str | None] = {}
    if "api_version" in step:
        params["api_version"] = step["api_version"]
    checked: AzureOpenAI | AsyncAzureOpenAI | None = None
    if "custom_query" in step:
        client = QueryClient(dict(step["custom_query"]))
        checked = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L117-L118
            AzureOpenAI | AsyncAzureOpenAI, client
        )
    try:
        changed = azure_module._check_dynamic_azure_params(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L115-L133
            params, checked
        )
    except KeyError:
        return {"error": "KeyError"}
    return {"changed": changed}
