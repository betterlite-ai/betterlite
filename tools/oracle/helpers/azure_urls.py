import os
from collections.abc import Generator
from contextlib import contextmanager
from typing import NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.azure import common_utils
from litellm.llms.azure.common_utils import BaseAzureLLM, get_azure_credentials
from litellm.types.router import GenericLiteLLMParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

ENV_KEYS: tuple[tuple[str, str], ...] = (
    ("tenant_id", "AZURE_TENANT_ID"),
    ("client_id", "AZURE_CLIENT_ID"),
    ("client_secret", "AZURE_CLIENT_SECRET"),
    ("azure_username", "AZURE_USERNAME"),
    ("azure_password", "AZURE_PASSWORD"),
    ("azure_scope", "AZURE_SCOPE"),
)


class CredentialParams(TypedDict, total=False):
    api_base: str
    api_key: str
    api_version: str


class Globals(TypedDict, total=False):
    api_base: str
    api_key: str
    azure_key: str
    api_version: str


class CredentialStep(TypedDict):
    params: CredentialParams
    globals: Globals
    environment: dict[str, str]


class UrlStep(TypedDict):
    params: CredentialParams
    globals: Globals
    environment: dict[str, str]
    route: str
    default_api_version: NotRequired[str]


class DeploymentStep(TypedDict):
    azure_endpoint: NotRequired[str]
    base_url: NotRequired[str]


class EnvironmentStep(TypedDict):
    params: dict[str, str | None]
    environment: dict[str, str]


class HeaderStep(TypedDict):
    headers: dict[str, str]
    params: CredentialParams
    globals: Globals
    environment: dict[str, str]
    token_params: dict[str, str]


class ImageStep(TypedDict):
    api_base: str
    api_version: NotRequired[str]
    route: str


class Input(TypedDict, total=False):
    credential_precedence: list[CredentialStep]
    deployment_base_url: list[DeploymentStep]
    v1_latest_preview: list[UrlStep]
    query_api_version_wins: list[UrlStep]
    append_route_once: list[UrlStep]
    azure_header_selection: list[HeaderStep]
    azure_env_none_vs_empty: list[EnvironmentStep]
    v1_image_resource_reset: list[ImageStep]


@contextmanager
def environment(values: dict[str, str]) -> Generator[None]:
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(values)
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(saved)


def apply_globals(globals_value: Globals) -> None:
    litellm.api_base = globals_value.get("api_base")
    litellm.api_key = globals_value.get("api_key")
    litellm.azure_key = globals_value.get("azure_key")
    litellm.api_version = globals_value.get("api_version")


def credential_report(step: CredentialStep) -> Json:
    params = step["params"]
    apply_globals(step["globals"])
    with environment(step["environment"]):
        resolved = get_azure_credentials(
            api_base=params.get("api_base"),
            api_key=params.get("api_key"),
            api_version=params.get("api_version"),
        )
    return {
        "api_base": resolved.api_base,
        "api_version": resolved.api_version,
        "api_key": resolved.api_key,
    }


def deployment_report(step: DeploymentStep) -> Json:
    params: dict[str, str] = {}
    azure_endpoint = step.get("azure_endpoint")
    if azure_endpoint is not None:
        params["azure_endpoint"] = azure_endpoint
    base_url = step.get("base_url")
    if base_url is not None:
        params["base_url"] = base_url
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L287-L296
        Json,
        common_utils.select_azure_base_url_or_endpoint(params),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L287-L296
    )


def url_value(step: UrlStep) -> str:
    params = step["params"]
    apply_globals(step["globals"])
    with environment(step["environment"]):
        return BaseAzureLLM._get_base_azure_url(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L780-L830
            api_base=params.get("api_base"),
            litellm_params={"api_version": params.get("api_version")},
            route=step["route"],
            default_api_version=step.get("default_api_version"),
        )


def url_report(step: UrlStep) -> Json:
    url = url_value(step)
    return {"url": url}


def is_v1(step: UrlStep) -> bool:
    version = step["params"].get("api_version")
    return BaseAzureLLM._is_azure_v1_api_version(version)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L858-L862


def version_report(step: UrlStep) -> Json:
    v1 = is_v1(step)
    return {"is_v1": v1, "url": url_value(step)}


def header_report(step: HeaderStep) -> Json:
    params = step["params"]
    tokens = step["token_params"]
    apply_globals(step["globals"])
    litellm.enable_azure_ad_token_refresh = False
    client = GenericLiteLLMParams(
        api_key=params.get("api_key"),
        azure_ad_token=tokens.get("azure_ad_token"),
        tenant_id=tokens.get("tenant_id"),
        client_id=tokens.get("client_id"),
        client_secret=tokens.get("client_secret"),
        azure_scope=tokens.get("azure_scope"),
        azure_username=tokens.get("azure_username"),
        azure_password=tokens.get("azure_password"),
    )
    with environment(step["environment"]):
        headers = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L752-L778
            "dict[str, str]",
            BaseAzureLLM._base_validate_azure_environment(dict(step["headers"]), client),  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L752-L778
        )
    return {"headers": cast(Json, headers)}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L752-L778


def env_report(step: EnvironmentStep) -> Json:
    azure = BaseAzureLLM()
    output: dict[str, str | None] = {}
    with environment(step["environment"]):
        for param, variable in ENV_KEYS:
            output[param] = azure._resolve_env_var(step["params"], param, variable)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L864-L876
    return cast(Json, output)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L864-L876


def image_report(step: ImageStep) -> Json:
    url = BaseAzureLLM.get_azure_v1_image_url(
        step["api_base"],
        step.get("api_version"),
        step["route"],
    )
    return {"url": url}


def run(input_value: Input) -> Json:
    if "credential_precedence" in input_value:
        result: Json = {
            "steps": [credential_report(step) for step in input_value["credential_precedence"]]
        }
    elif "deployment_base_url" in input_value:
        result = {
            "steps": [deployment_report(step) for step in input_value["deployment_base_url"]]
        }
    elif "v1_latest_preview" in input_value:
        result = {"steps": [version_report(step) for step in input_value["v1_latest_preview"]]}
    elif "query_api_version_wins" in input_value:
        result = {"steps": [url_report(step) for step in input_value["query_api_version_wins"]]}
    elif "append_route_once" in input_value:
        result = {"steps": [url_report(step) for step in input_value["append_route_once"]]}
    elif "azure_header_selection" in input_value:
        result = {
            "steps": [header_report(step) for step in input_value["azure_header_selection"]]
        }
    elif "azure_env_none_vs_empty" in input_value:
        result = {
            "steps": [env_report(step) for step in input_value["azure_env_none_vs_empty"]]
        }
    elif "v1_image_resource_reset" in input_value:
        result = {
            "steps": [image_report(step) for step in input_value["v1_image_resource_reset"]]
        }
    else:
        raise ValueError("expected one azure_urls variant")
    return result
