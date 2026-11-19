import os
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.deepseek.messages.transformation import DeepSeekAnthropicMessagesConfig
from litellm.llms.minimax.messages.transformation import MinimaxMessagesConfig
from litellm.llms.tencent.messages.transformation import TencentAnthropicMessagesConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue as Json

JsonObject: TypeAlias = dict[str, Json]
Headers: TypeAlias = dict[str, Json]
HostName: TypeAlias = Literal["deepseek", "tencent", "minimax"]
MODEL: Final = "claude-sonnet-4-5"
NO_PARAMS: Final[JsonObject] = {}


class Config(Protocol):
    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: JsonObject,
        litellm_params: JsonObject,
    ) -> str: ...

    def validate_anthropic_messages_environment(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/deepseek/messages/transformation.py#L45-L72
        self,
        headers: Headers,
        model: str,
        messages: list[Json],
        optional_params: JsonObject,
        litellm_params: JsonObject,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> tuple[JsonObject, str | None]: ...

    def transform_anthropic_messages_request(
        self,
        model: str,
        messages: list[JsonObject],
        anthropic_messages_optional_request_params: JsonObject,
        litellm_params: GenericLiteLLMParams,
        headers: JsonObject,
    ) -> JsonObject: ...


class UrlStep(TypedDict):
    function: Literal["url"]
    host: HostName
    api_bases: list[str | None]
    environment: dict[str, str]


class EnvironmentStep(TypedDict):
    function: Literal["environment"]
    host: HostName
    api_base: str | None
    headers: Headers
    optional_params: JsonObject
    messages: list[Json]
    explicit_api_key: str | None
    global_api_key: str | None
    environment: dict[str, str]


class TransformStep(TypedDict):
    function: Literal["transform"]
    host: HostName
    model: str
    messages: list[JsonObject]
    max_tokens: int
    system: Json | None
    tools: Json | None


class Input(TypedDict):
    calls: list[UrlStep | EnvironmentStep | TransformStep]


def config_for(host: HostName) -> Config:
    if host == "deepseek":
        return DeepSeekAnthropicMessagesConfig()
    if host == "tencent":
        return TencentAnthropicMessagesConfig()
    return MinimaxMessagesConfig()


def use_environment(values: dict[str, str], global_api_key: str | None) -> None:
    os.environ.clear()
    os.environ.update(values)
    litellm.api_key = global_api_key


def url(step: UrlStep) -> Json:
    use_environment(step["environment"], None)
    config = config_for(step["host"])
    return {
        "urls": [
            config.get_complete_url(api_base, None, MODEL, NO_PARAMS, NO_PARAMS)
            for api_base in step["api_bases"]
        ]
    }


def environment(step: EnvironmentStep) -> Json:
    use_environment(step["environment"], step["global_api_key"])
    config = config_for(step["host"])
    headers, api_base = config.validate_anthropic_messages_environment(
        step["headers"].copy(),
        MODEL,
        step["messages"],
        step["optional_params"],
        NO_PARAMS,
        api_key=step["explicit_api_key"],
        api_base=step["api_base"],
    )
    return {"headers": headers, "api_base": api_base}


def transform(step: TransformStep) -> Json:
    use_environment({}, None)
    config = config_for(step["host"])
    params: JsonObject = {"max_tokens": step["max_tokens"]}
    system = step["system"]
    if system is not None:
        params["system"] = system
    tools = step["tools"]
    if tools is not None:
        params["tools"] = tools
    transformed = config.transform_anthropic_messages_request(
        step["model"], step["messages"], params, GenericLiteLLMParams(), NO_PARAMS
    )
    return {
        "system": transformed.get("system"),
        "messages": transformed.get("messages"),
        "tools": transformed.get("tools"),
    }


def run(argument: Input) -> Json:
    results: list[Json] = []
    for step in argument["calls"]:
        match step["function"]:
            case "url":
                results.append(url(step))
            case "environment":
                results.append(environment(step))
            case "transform":
                results.append(transform(step))
    return {"calls": results}
