import os
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.anthropic.pass_through.messages.transformation import (
    AnthropicMessagesConfig,
)
from litellm.llms.azure_ai.anthropic.messages_transformation import (
    AzureAnthropicMessagesConfig,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue as Json
from typing_extensions import override

JsonObject: TypeAlias = dict[str, Json]
MODEL: Final = "claude-sonnet-4-5"
NO_PARAMS: Final[JsonObject] = {}


class Config(Protocol):
    @property
    def custom_llm_provider(self) -> str | None: ...

    def should_strip_billing_metadata(self) -> bool: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: JsonObject,
        litellm_params: JsonObject,
    ) -> str: ...

    def validate_anthropic_messages_environment(  # ast-grep-ignore: no-long-names
        self,
        headers: JsonObject,
        model: str,
        messages: list[Json],
        optional_params: JsonObject,
        litellm_params: JsonObject,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> tuple[JsonObject, str | None]: ...

    def transform_anthropic_messages_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        messages: list[JsonObject],
        anthropic_messages_optional_request_params: JsonObject,  # ast-grep-ignore: no-long-names
        litellm_params: GenericLiteLLMParams,
        headers: JsonObject,
    ) -> JsonObject: ...


# B961 owns the parent request transform and system normalization, so the seam hands back what it was given
class Seam(AnthropicMessagesConfig):
    @override
    def transform_anthropic_messages_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        messages: list[JsonObject],
        anthropic_messages_optional_request_params: JsonObject,  # ast-grep-ignore: no-long-names
        litellm_params: GenericLiteLLMParams,
        headers: JsonObject,
    ) -> JsonObject:
        return anthropic_messages_optional_request_params

    @override
    def _normalize_system_role_messages(  # ast-grep-ignore: no-long-names
        self,
        anthropic_messages_request: JsonObject,
        model: str,
    ) -> None:
        return None


class Driver(AzureAnthropicMessagesConfig, Seam):
    pass


class Globals(TypedDict):
    api_key: str | None
    azure_key: str | None


class UrlCall(TypedDict):
    function: Literal["url"]
    api_bases: list[str | None]
    environment: dict[str, str]


class EnvironmentStep(TypedDict):
    headers: JsonObject
    litellm_params: JsonObject
    api_key: str | None
    api_base: str | None
    optional_params: JsonObject
    messages: list[Json]
    globals: Globals
    environment: dict[str, str]


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    calls: list[EnvironmentStep]


class TransformCall(TypedDict):
    function: Literal["transform"]
    request: JsonObject


class ProjectionCall(TypedDict):
    function: Literal["projection"]
    request: JsonObject


Input: TypeAlias = UrlCall | EnvironmentCall | TransformCall | ProjectionCall


def use_environment(values: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(values)


def url(config: Config, call: UrlCall) -> Json:
    use_environment(call["environment"])
    urls: list[Json] = [
        config.get_complete_url(api_base, None, MODEL, NO_PARAMS, NO_PARAMS)
        for api_base in call["api_bases"]
    ]
    return {"urls": urls}


def environment_answer(config: Config, step: EnvironmentStep) -> Json:
    use_environment(step["environment"])
    litellm.api_key = step["globals"]["api_key"]
    litellm.azure_key = step["globals"]["azure_key"]
    litellm.enable_azure_ad_token_refresh = False
    headers, api_base = config.validate_anthropic_messages_environment(
        step["headers"].copy(),
        MODEL,
        step["messages"],
        step["optional_params"],
        step["litellm_params"],
        step["api_key"],
        step["api_base"],
    )
    return {"headers": headers, "api_base": api_base}


def transformed(config: Config, request: JsonObject) -> JsonObject:
    return config.transform_anthropic_messages_request(
        MODEL, [], request, GenericLiteLLMParams(), NO_PARAMS
    )


def run(argument: Input) -> Json:
    config: Config = Driver()
    match argument["function"]:
        case "url":
            return url(config, argument)
        case "environment":
            answers: list[Json] = [
                environment_answer(config, step) for step in argument["calls"]
            ]
            return {"calls": answers}
        case "transform":
            return {"request": transformed(config, argument["request"])}
        case "projection":
            return {
                "custom_llm_provider": config.custom_llm_provider,
                "should_strip_billing_metadata": config.should_strip_billing_metadata(),
                "request": transformed(config, argument["request"]),
            }
