import os

from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from pydantic import JsonValue

from .views import (
    ChatView,
    EmbeddingView,
    HeadersStep,
    InfoStep,
    ParamsConfig,
    ParamsStep,
    UrlConfig,
    UrlStep,
)


def set_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


def url_step(chat: ChatView, embed: EmbeddingView, step: UrlStep) -> JsonValue:
    set_env("SNOWFLAKE_ACCOUNT_ID", step["account_env"])
    params = dict(step["params"])
    config: UrlConfig = embed if step["embedding"] else chat
    try:
        url = config.get_complete_url(
            api_base=step["api_base"],
            api_key=None,
            model=step["model"],
            optional_params=params,
            litellm_params={},
        )
    except ValueError as error:
        return {"error": str(error)}
    return {"url": url, "params": params}


def headers_step(chat: ChatView, step: HeadersStep) -> JsonValue:
    try:
        headers = chat.validate_environment(
            headers=dict(step["headers"]),
            model=step["model"],
            messages=[],
            optional_params={},
            litellm_params={},
            api_key=step["api_key"],
        )
    except ValueError as error:
        return {"error": str(error)}
    return {"headers": headers}


def params_step(chat: ChatView, embed: EmbeddingView, step: ParamsStep) -> JsonValue:
    config: ParamsConfig = embed if step["embedding"] else chat
    mapped = config.map_openai_params(
        non_default_params=step["non_default"],
        optional_params={},
        model=step["model"],
        drop_params=False,
    )
    return {"supported": config.get_supported_openai_params(model=step["model"]), "mapped": mapped}


def info_step(step: InfoStep) -> JsonValue:
    set_env("SNOWFLAKE_JWT", step["jwt_env"])
    _, _, api_key, api_base = get_llm_provider(
        model="cortex-model",
        custom_llm_provider="snowflake",
        api_base=step["api_base"],
        api_key=step["api_key"],
    )
    return {"api_base": api_base, "api_key": api_key}
