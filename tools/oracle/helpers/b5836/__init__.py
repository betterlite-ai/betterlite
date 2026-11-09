import os
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.llms.cloudflare.chat.transformation import CloudflareChatConfig
from litellm.llms.cloudflare.decisions.transformation import CloudflareDecisionsConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

MODEL = "cloudflare/llama"


def chat_url(call: dict[str, Json]) -> Json:
    api_base = cast(str | None, call.get("api_base"))
    env = cast(dict[str, str], call.get("env") or {})
    with patch.dict(os.environ, env, clear=True):
        try:
            url: str = CloudflareChatConfig().get_complete_url(  # pyright: ignore[reportUnknownMemberType]
                api_base=api_base,
                api_key=None,
                model=MODEL,
                optional_params={},
                litellm_params={},
            )
        except Exception as error:  # noqa: BLE001 - LiteLLM raises ValueError for these inputs
            return {"error": str(error)}
    return {"url": url}


def chat_headers(call: dict[str, Json]) -> Json:
    api_key = cast(str | None, call.get("api_key"))
    headers = cast(dict[str, str], call.get("headers") or {})
    try:
        validated: Json = CloudflareChatConfig().validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            headers=dict(headers),
            model=MODEL,
            messages=[],
            optional_params={},
            litellm_params={},
            api_key=api_key,
        )
    except Exception as error:  # noqa: BLE001 - LiteLLM raises ValueError when the key is None
        return {"error": str(error)}
    return {"headers": validated}


def chat_error(call: dict[str, Json]) -> Json:
    message = cast(str, call["message"])
    status_code = cast(int, call["status_code"])
    headers = cast(dict[str, str], call.get("headers") or {})
    error = CloudflareChatConfig().get_error_class(  # pyright: ignore[reportUnknownMemberType]
        message, status_code, dict(headers)
    )
    return {
        "class": type(error).__name__,
        "status_code": error.status_code,
        "message": error.message,
        "request_method": error.request.method,
        "request_url": str(error.request.url),
        "response_status": error.response.status_code,
        "headers": cast(Json, error.headers),
    }


def decisions_defaults(call: dict[str, Json]) -> Json:
    env = cast(dict[str, str], call.get("env") or {})
    with patch.dict(os.environ, env, clear=True):
        config = CloudflareDecisionsConfig()
        return {
            "api_key_env": list(config.api_key_env),
            "api_base_env": list(config.api_base_env),
            "default_base": config.get_default_api_base(),
            "missing_base_message": config.missing_api_base_message("cloudflare"),
        }


def decisions_model(model: str) -> Json:
    config = CloudflareDecisionsConfig()
    return {"canonical": config.canonical_model(model), "request": config.request_model(model)}


def decisions_url(call: dict[str, Json]) -> Json:
    config = CloudflareDecisionsConfig()
    api_base = cast(str, call["api_base"])
    model = cast(str, call["model"])
    return {"url": config.get_complete_url(api_base, config.canonical_model(model))}


def unwrap(payload: Json) -> Json:
    config = CloudflareDecisionsConfig()
    return cast(Json, config.unwrap_response(payload))


def run(value: dict[str, Json]) -> Json:
    urls = cast(list[dict[str, Json]], value.get("chat_urls") or [])
    headers = cast(list[dict[str, Json]], value.get("chat_headers") or [])
    errors = cast(list[dict[str, Json]], value.get("chat_errors") or [])
    defaults = cast(list[dict[str, Json]], value.get("decisions_defaults") or [])
    models = cast(list[str], value.get("decisions_models") or [])
    decisions = cast(list[dict[str, Json]], value.get("decisions_urls") or [])
    payloads = cast(list[Json], value.get("unwrap") or [])
    return {
        "chat_urls": [chat_url(call) for call in urls],
        "chat_headers": [chat_headers(call) for call in headers],
        "chat_errors": [chat_error(call) for call in errors],
        "decisions_defaults": [decisions_defaults(call) for call in defaults],
        "decisions_models": [decisions_model(model) for model in models],
        "decisions_urls": [decisions_url(call) for call in decisions],
        "unwrap": [unwrap(payload) for payload in payloads],
    }
