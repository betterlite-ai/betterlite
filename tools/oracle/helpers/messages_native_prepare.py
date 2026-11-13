import os
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.llms.anthropic.pass_through.messages.transformation import (
    AnthropicMessagesConfig,
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.vertex_ai.vertex_ai_partner_models.anthropic.experimental_pass_through import (
    transformation as vertex_messages,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue as Json
from typing_extensions import override

JsonObject: TypeAlias = dict[str, Json]
Headers: TypeAlias = dict[str, Json]
MODEL: Final = "claude-sonnet-4-5"
NO_PARAMS: Final[JsonObject] = {}
NO_MESSAGES: Final[list[Json]] = []
NO_HEADERS: Final[Headers] = {}


class Config(Protocol):
    @property
    def max_retry_on_anthropic_messages_http_error(  # ast-grep-ignore: no-long-names
        self,
    ) -> int: ...

    def get_supported_anthropic_messages_params(  # ast-grep-ignore: no-long-names
        self, model: str
    ) -> list[Json]: ...

    def should_filter_anthropic_beta_headers(  # ast-grep-ignore: no-long-names
        self,
    ) -> bool: ...

    def should_strip_billing_metadata(self) -> bool: ...

    def handles_web_search_natively(self) -> bool: ...

    def uses_get_llm_provider_api_base(self) -> bool: ...

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
    ) -> tuple[JsonObject, str | None]: ...

    def sign_request(
        self,
        headers: JsonObject,
        optional_params: JsonObject,
        request_data: JsonObject,
        api_base: str,
    ) -> tuple[JsonObject, bytes | None]: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: JsonObject | httpx.Headers
    ) -> BaseLLMException: ...

    def should_retry_anthropic_messages_on_http_error(  # ast-grep-ignore: no-long-names
        self, e: httpx.HTTPStatusError, litellm_params: JsonObject
    ) -> bool: ...

    def transform_anthropic_messages_request_on_http_error(  # ast-grep-ignore: no-long-names
        self, e: httpx.HTTPStatusError, request_data: JsonObject
    ) -> JsonObject: ...

    def transform_anthropic_messages_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        messages: list[JsonObject],
        anthropic_messages_optional_request_params: JsonObject,  # ast-grep-ignore: no-long-names
        litellm_params: GenericLiteLLMParams,
        headers: JsonObject,
    ) -> JsonObject: ...


class BedrockConfig(AnthropicMessagesConfig):
    @property
    @override
    def custom_llm_provider(self) -> str | None:
        return "bedrock"


class NullConfig(AnthropicMessagesConfig):
    @property
    @override
    def custom_llm_provider(self) -> str | None:
        return None


# Vertex's transform publicly calls the parent's normalization and scope removal
class DriverConfig(vertex_messages.VertexAIPartnerModelsAnthropicMessagesConfig):
    @property
    @override
    def custom_llm_provider(self) -> str | None:
        return None


def config_for(provider: str | None) -> Config:
    match provider:
        case None:
            return NullConfig()
        case "anthropic":
            return AnthropicMessagesConfig()
        case "bedrock":
            return BedrockConfig()
        case _:
            raise ValueError(f"unknown provider {provider}")


class SupportedFieldsCall(TypedDict):
    function: Literal["supported_fields"]
    providers: list[str | None]


class UrlCall(TypedDict):
    function: Literal["url"]
    api_bases: list[str | None]
    environment: dict[str, str]


class EnvironmentStep(TypedDict):
    headers: Headers
    environment: dict[str, str]


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    calls: list[EnvironmentStep]


class TransformExtras(TypedDict, total=False):
    system: Json
    tools: list[Json]


class TransformBody(TransformExtras):
    model: str
    max_tokens: int
    messages: list[JsonObject]


class TransformStep(TypedDict):
    request: TransformBody
    supports_mid_system: bool


class TransformCall(TypedDict):
    function: Literal["transform"]
    calls: list[TransformStep]


class BaseDefaultsCall(TypedDict):
    function: Literal["base_defaults"]
    headers: Headers
    status: int
    text: str
    request: JsonObject
    error_message: str
    error_status: int


Input: TypeAlias = (
    SupportedFieldsCall
    | UrlCall
    | EnvironmentCall
    | TransformCall
    | BaseDefaultsCall
)


def field_projection(config: Config) -> Json:
    projection: JsonObject = {
        "fields": config.get_supported_anthropic_messages_params(MODEL),
        "filters_betas": config.should_filter_anthropic_beta_headers(),
        "strips_billing": config.should_strip_billing_metadata(),
    }
    return projection


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
    headers, api_base = config.validate_anthropic_messages_environment(
        step["headers"].copy(),
        MODEL,
        NO_MESSAGES,
        NO_PARAMS,
        NO_PARAMS,
    )
    return {"headers": headers, "api_base": api_base}


def environment(config: Config, call: EnvironmentCall) -> Json:
    answers: list[Json] = [environment_answer(config, step) for step in call["calls"]]
    return {"calls": answers}


def transform_answer(config: Config, step: TransformStep) -> Json:
    body = step["request"]
    params: JsonObject = {"max_tokens": body["max_tokens"]}
    system = body.get("system")
    if system is not None:
        params["system"] = system
    tools = body.get("tools")
    if tools is not None:
        params["tools"] = tools
    transformed = config.transform_anthropic_messages_request(
        body["model"], body["messages"], params, GenericLiteLLMParams(), NO_PARAMS
    )
    return {
        "system": transformed.get("system"),
        "messages": transformed.get("messages"),
        "tools": transformed.get("tools"),
    }


def transform(config: Config, call: TransformCall) -> Json:
    answers: list[Json] = [transform_answer(config, step) for step in call["calls"]]
    return {"calls": answers}


def http_status_error(status: int, text: str) -> httpx.HTTPStatusError:
    request = httpx.Request("POST", "https://api.anthropic.com/v1/messages")
    response = httpx.Response(status, text=text, request=request)
    return httpx.HTTPStatusError("retry decision", request=request, response=response)


def base_defaults(config: Config, call: BaseDefaultsCall) -> Json:
    error = config.get_error_class(call["error_message"], call["error_status"], NO_HEADERS)
    http_error = http_status_error(call["status"], call["text"])
    signed = config.sign_request(
        call["headers"].copy(), NO_PARAMS, NO_PARAMS, "https://api.anthropic.com"
    )
    retried: JsonObject = config.transform_anthropic_messages_request_on_http_error(
        http_error, call["request"].copy()
    )
    projection: JsonObject = {
        "signed": {"headers": signed[0], "has_signed_body": signed[1] is not None},
        "url": config.get_complete_url(None, None, MODEL, NO_PARAMS, NO_PARAMS),
        "web_search": config.handles_web_search_natively(),
        "uses_provider_base": config.uses_get_llm_provider_api_base(),
        "filters_betas": config.should_filter_anthropic_beta_headers(),
        "error": {"status": error.status_code, "message": error.message},
        "max_retries": config.max_retry_on_anthropic_messages_http_error,
        "retried": config.should_retry_anthropic_messages_on_http_error(http_error, NO_PARAMS),
        "request": retried,
    }
    return projection


def run(argument: Input) -> Json:
    match argument["function"]:
        case "supported_fields":
            return {
                "providers": [
                    field_projection(config_for(provider)) for provider in argument["providers"]
                ]
            }
        case "url":
            return url(AnthropicMessagesConfig(), argument)
        case "environment":
            return environment(AnthropicMessagesConfig(), argument)
        case "transform":
            return transform(DriverConfig(), argument)
        case "base_defaults":
            return base_defaults(BedrockConfig(), argument)
