from dataclasses import dataclass
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

import litellm
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.types.videos.main import VideoCreateOptionalRequestParams as VideoOptions
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue

from azure_urls import environment

MODEL = "sora-2"

Resource = Literal["container", "video", "vector_store"]
Report: TypeAlias = dict[str, JsonValue]
Reports: TypeAlias = dict[str, list[Report]]


class ContainerConfig(Protocol):
    def validate_environment(
        self, headers: dict[str, str], api_key: str | None = None
    ) -> Report: ...

    def get_complete_url(self, api_base: str | None, litellm_params: dict[str, str]) -> str: ...


class VideoConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[JsonValue]: ...

    def map_openai_params(
        self,
        video_create_optional_params: VideoOptions,
        model: str,
        *,
        drop_params: bool,
    ) -> Report: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        api_key: str | None = None,
        litellm_params: GenericLiteLLMParams | None = None,
    ) -> Report: ...

    def get_complete_url(
        self, model: str, api_base: str | None, litellm_params: dict[str, str]
    ) -> str: ...


class VectorStoreConfig(Protocol):
    def get_complete_url(self, api_base: str | None, litellm_params: dict[str, str]) -> str: ...

    def validate_environment(
        self, headers: dict[str, str], litellm_params: GenericLiteLLMParams | None
    ) -> Report: ...


@dataclass(frozen=True)
class Configs:
    container: ContainerConfig
    video: VideoConfig
    vector_store: VectorStoreConfig


class Module(TypedDict, total=False):
    api_base: str
    api_key: str
    azure_key: str


class UrlStep(TypedDict):
    api_base: NotRequired[str]
    api_version: NotRequired[str]
    module: Module
    environment: dict[str, str]


class Params(TypedDict, total=False):
    api_key: str
    azure_ad_token: str


class AuthStep(TypedDict):
    resource: Resource
    headers: dict[str, str]
    api_key: NotRequired[str]
    params: NotRequired[Params]
    module: Module
    environment: dict[str, str]


class VideoStep(TypedDict):
    options: VideoOptions
    drop_params: bool


class UrlsAndAuth(TypedDict):
    urls: list[UrlStep]
    auth: list[AuthStep]


class Input(TypedDict, total=False):
    container_api_version_query: list[UrlStep]
    container_normalized_base: list[UrlStep]
    video_supported_fields: list[VideoStep]
    video_auth_and_url: UrlsAndAuth
    vector_search_azure_base: UrlsAndAuth


def apply_module(module: Module) -> None:
    litellm.api_base = module.get("api_base")
    litellm.api_key = module.get("api_key")
    litellm.azure_key = module.get("azure_key")


def url_report(configs: Configs, resource: Resource, step: UrlStep) -> Report:
    params: dict[str, str] = {}
    api_version = step.get("api_version")
    if api_version is not None:
        params["api_version"] = api_version
    api_base = step.get("api_base")
    apply_module(step["module"])
    with environment(step["environment"]):
        if resource == "container":
            url = configs.container.get_complete_url(api_base, params)
        elif resource == "video":
            url = configs.video.get_complete_url(MODEL, api_base, params)
        else:
            url = configs.vector_store.get_complete_url(api_base, params)
        return {"url": url, "params_api_version": params.get("api_version")}


def auth_report(configs: Configs, step: AuthStep) -> Report:
    params = step.get("params")
    supplied = (
        None
        if params is None
        else GenericLiteLLMParams(
            api_key=params.get("api_key"), azure_ad_token=params.get("azure_ad_token")
        )
    )
    apply_module(step["module"])
    litellm.enable_azure_ad_token_refresh = False
    headers = dict(step["headers"])
    resource = step["resource"]
    with environment(step["environment"]):
        if resource == "container":
            result = configs.container.validate_environment(headers, step.get("api_key"))
        elif resource == "video":
            result = configs.video.validate_environment(
                headers, MODEL, step.get("api_key"), supplied
            )
        else:
            result = configs.vector_store.validate_environment(headers, supplied)
        return {"headers": result, "params_api_key": None if supplied is None else supplied.api_key}


def video_fields_report(configs: Configs, step: VideoStep) -> Report:
    video = configs.video
    return {
        "supported": video.get_supported_openai_params(MODEL),
        "mapped": video.map_openai_params(step["options"], MODEL, drop_params=step["drop_params"]),
    }


def urls_and_auth_report(configs: Configs, resource: Resource, steps: UrlsAndAuth) -> Reports:
    return {
        "urls": [url_report(configs, resource, step) for step in steps["urls"]],
        "auth": [auth_report(configs, step) for step in steps["auth"]],
    }


def azure_configs() -> Configs:
    container = ProviderConfigManager.get_provider_container_config(LlmProviders.AZURE)
    video = ProviderConfigManager.get_provider_video_config(MODEL, LlmProviders.AZURE)
    vector_store = ProviderConfigManager.get_provider_vector_stores_config(LlmProviders.AZURE)
    if container is None or video is None or vector_store is None:
        raise ValueError("litellm has no azure container, video or vector store config")
    return Configs(container, video, vector_store)


def run(input_value: Input) -> Reports:
    configs = azure_configs()
    if "container_api_version_query" in input_value:
        queries = input_value["container_api_version_query"]
        return {"steps": [url_report(configs, "container", step) for step in queries]}
    if "container_normalized_base" in input_value:
        bases = input_value["container_normalized_base"]
        return {"steps": [url_report(configs, "container", step) for step in bases]}
    if "video_supported_fields" in input_value:
        videos = input_value["video_supported_fields"]
        return {"steps": [video_fields_report(configs, step) for step in videos]}
    if "video_auth_and_url" in input_value:
        return urls_and_auth_report(configs, "video", input_value["video_auth_and_url"])
    if "vector_search_azure_base" in input_value:
        search = input_value["vector_search_azure_base"]
        return urls_and_auth_report(configs, "vector_store", search)
    raise ValueError("expected one azure_resource_urls variant")
