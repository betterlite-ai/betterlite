from collections.abc import Callable
from typing import TypeAlias, cast

import litellm
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import LlmProviders, ModelResponse, TextCompletionResponse
from litellm.utils import ProviderConfigManager

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected object")
    return value

def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value

class _FreshTextCompletionConfig(litellm.OpenAITextCompletionConfig):
    pass

def config_values(fields: Object) -> Json:
    settings = obj(fields.get("settings", {}))
    config: dict[str, object] = cast(
        dict[str, object],
        _FreshTextCompletionConfig(**settings).get_config(),  # pyright: ignore[reportUnknownMemberType, reportArgumentType]
    )
    return cast(Json, config)

def parameter_allowlist(fields: Object) -> Json:
    result: list[object] = cast(
        list[object],
        litellm.OpenAITextCompletionConfig().get_supported_openai_params(  # pyright: ignore[reportUnknownMemberType]
            model=text(fields, "model")
        ),
    )
    return cast(Json, result)

def prompt_request(fields: Object) -> Json:
    messages = fields.get("messages", [])
    optional_params = obj(fields.get("optional_params", {}))
    result: dict[str, object] = cast(
        dict[str, object],
        litellm.OpenAITextCompletionConfig().transform_text_completion_request(  # pyright: ignore[reportUnknownMemberType]
            model=text(fields, "model"),
            messages=cast(list[AllMessageValues], messages),
            optional_params=optional_params,
            headers={},
        ),
    )
    return cast(Json, result)

def reply_as_chat(fields: Object) -> Json:
    response_value = fields.get("response_object")
    target_value = fields.get("model_response_object")
    if response_value is None or target_value is None:
        raise ValueError("Error in response object format")
    response_object = TextCompletionResponse(**obj(response_value))  # pyright: ignore[reportArgumentType]
    model_response_object = ModelResponse(**obj(target_value))
    converted = litellm.OpenAITextCompletionConfig().convert_to_chat_model_response_object(  # pyright: ignore[reportUnknownMemberType]
        response_object=response_object, model_response_object=model_response_object
    )
    dumped: dict[str, object] = cast(
        dict[str, object],
        converted.model_dump(),  # pyright: ignore[reportUnknownMemberType]
    )
    dumped["_hidden_params"] = {"original_response": cast(object, response_object.model_dump())}
    return cast(Json, dumped)

PROVIDER_TAGS: dict[str, str] = {
    "OpenAITextCompletionConfig": "open_ai",
    "FireworksAITextCompletionConfig": "fireworks_ai",
    "TogetherAITextCompletionConfig": "together_ai",
    "InceptionTextCompletionConfig": "text_completion_inception",
}

PROVIDERS_BY_NAME: dict[str, LlmProviders] = {
    "fireworks_ai": LlmProviders.FIREWORKS_AI,
    "together_ai": LlmProviders.TOGETHER_AI,
    "text_completion_inception": LlmProviders.TEXT_COMPLETION_INCEPTION,
}

def config_selection(fields: Object) -> Json:
    provider = PROVIDERS_BY_NAME.get(text(fields, "provider"), LlmProviders.OPENAI)
    config = ProviderConfigManager.get_provider_text_completion_config(
        model="any-model", provider=provider
    )
    return PROVIDER_TAGS[type(config).__name__]

HANDLERS: dict[str, Callable[[Object], Json]] = {
    "text_config_values": config_values,
    "text_parameter_allowlist": parameter_allowlist,
    "single_prompt_tokens": prompt_request,
    "multiple_prompt_strings": prompt_request,
    "text_reply_as_chat": reply_as_chat,
    "text_config_selection": config_selection,
}

def run(argument: Json) -> Json:
    fields = obj(argument)
    if len(fields) != 1:
        raise ValueError("expected one text_completion_policy variant")
    name, value = next(iter(fields.items()))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown text_completion_policy case: {name}")
    return handler(obj(value))
