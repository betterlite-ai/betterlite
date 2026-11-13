import os
from typing import Final, Literal, TypeAlias, TypedDict

import httpx
from litellm.exceptions import AuthenticationError
from litellm.llms.anthropic.pass_through.messages.transformation import (
    AnthropicMessagesConfig,
)
from pydantic import JsonValue as Json
from typing_extensions import override

JsonObject: TypeAlias = dict[str, Json]
Headers: TypeAlias = dict[str, Json]
MODEL: Final = "claude-sonnet-4-5"
NO_PARAMS: Final[JsonObject] = {}
NO_MESSAGES: Final[list[Json]] = []
NO_HEADERS: Final[Headers] = {}


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


def config_for(provider: str | None) -> AnthropicMessagesConfig:
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
    api_key: str | None
    api_base: str | None
    environment: dict[str, str]


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    calls: list[EnvironmentStep]


class BillingCall(TypedDict):
    function: Literal["billing"]
    systems: list[Json]
    request: JsonObject


class SystemStep(TypedDict):
    request: JsonObject
    model: str
    supports_mid_system: bool


class SystemCall(TypedDict):
    function: Literal["system"]
    calls: list[SystemStep]


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
    | BillingCall
    | SystemCall
    | BaseDefaultsCall
)


def field_projection(provider: str | None) -> Json:
    config = config_for(provider)
    fields: list[Json] = config.get_supported_anthropic_messages_params(MODEL)  # pyright: ignore[reportAny, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/transformation.py#L66
    projection: JsonObject = {
        "fields": fields,
        "filters_betas": config.should_filter_anthropic_beta_headers(),
        "strips_billing": config.should_strip_billing_metadata(),
    }
    return projection


def use_environment(values: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(values)


def url(call: UrlCall) -> Json:
    config = AnthropicMessagesConfig()
    use_environment(call["environment"])
    urls: list[Json] = [
        config.get_complete_url(api_base, None, MODEL, NO_PARAMS, NO_PARAMS)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/transformation.py#L232
        for api_base in call["api_bases"]
    ]
    return {"urls": urls}


def environment_answer(step: EnvironmentStep) -> Json:
    config = AnthropicMessagesConfig()
    given = step["headers"].copy()
    use_environment(step["environment"])
    try:
        answer: tuple[Headers, str | None] = config.validate_anthropic_messages_environment(  # pyright: ignore[reportAny, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/transformation.py#L246
            given,
            MODEL,
            NO_MESSAGES,
            NO_PARAMS,
            NO_PARAMS,
            api_key=step["api_key"],
            api_base=step["api_base"],
        )
    except AuthenticationError as error:  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/transformation.py#L259
        return {"error": {"status": error.status_code}}
    return {"headers": answer[0], "api_base": answer[1]}


def billing(call: BillingCall) -> Json:
    config = AnthropicMessagesConfig()
    request = dict(call["request"])
    config._remove_scope_from_cache_control(request)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/transformation.py#L94
    filtered: list[Json | None] = [
        config._filter_billing_headers_from_system(system)  # type: ignore[no-untyped-call, misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/transformation.py#L134
        for system in call["systems"]
    ]
    return {"systems": filtered, "request": request}


def system(call: SystemCall) -> Json:
    answers: list[Json] = [system_answer(step) for step in call["calls"]]
    return {"calls": answers}


def system_answer(step: SystemStep) -> Json:
    config = AnthropicMessagesConfig()
    request = dict(step["request"])
    config._normalize_system_role_messages(request, model=step["model"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/transformation.py#L167
    return {"request": request}


def http_status_error(status: int, text: str) -> httpx.HTTPStatusError:
    request = httpx.Request("POST", "https://api.anthropic.com/v1/messages")
    response = httpx.Response(status, text=text, request=request)
    return httpx.HTTPStatusError("retry decision", request=request, response=response)


def stream_error(config: AnthropicMessagesConfig) -> Json:
    try:
        config.get_async_streaming_response_iterator(MODEL, httpx.Response(200), NO_PARAMS, None)  # type: ignore[arg-type]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/anthropic_messages/transformation.py#L134
    except NotImplementedError as error:
        return str(error)
    return None


def base_defaults(call: BaseDefaultsCall) -> Json:
    config = BedrockConfig()
    error = config.get_error_class(call["error_message"], call["error_status"], NO_HEADERS)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/anthropic_messages/transformation.py#L143
    http_error = http_status_error(call["status"], call["text"])
    signed: tuple[Headers, bytes | None] = config.sign_request(  # pyright: ignore[reportAny, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/anthropic_messages/transformation.py#L87
        call["headers"].copy(), NO_PARAMS, NO_PARAMS, "https://api.anthropic.com"
    )
    retried: JsonObject = config.transform_anthropic_messages_request_on_http_error(  # pyright: ignore[reportAny, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/anthropic_messages/transformation.py#L170
        http_error, call["request"].copy()
    )
    projection: JsonObject = {
        "signed": {"headers": signed[0], "has_signed_body": signed[1] is not None},
        "url": config.get_complete_url(None, None, MODEL, NO_PARAMS, NO_PARAMS),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/transformation.py#L232
        "web_search": config.handles_web_search_natively(),
        "uses_provider_base": config.uses_get_llm_provider_api_base(),
        "filters_betas": config.should_filter_anthropic_beta_headers(),
        "stream_error": stream_error(config),
        "error": {"status": error.status_code, "message": error.message},
        "max_retries": config.max_retry_on_anthropic_messages_http_error,
        "retried": config.should_retry_anthropic_messages_on_http_error(http_error, NO_PARAMS),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/anthropic_messages/transformation.py#L159
        "request": retried,
    }
    return projection


def run(argument: Input) -> Json:
    match argument["function"]:
        case "supported_fields":
            return {
                "providers": [
                    field_projection(provider) for provider in argument["providers"]
                ]
            }
        case "url":
            return url(argument)
        case "environment":
            return {"calls": [environment_answer(step) for step in argument["calls"]]}
        case "billing":
            return billing(argument)
        case "system":
            return system(argument)
        case "base_defaults":
            return base_defaults(argument)
