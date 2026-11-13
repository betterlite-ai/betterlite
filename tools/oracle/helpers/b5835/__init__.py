import json
import os
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.vercel_ai_gateway.chat.transformation import VercelAIGatewayConfig
from litellm.llms.vercel_ai_gateway.embedding.transformation import VercelAIGatewayEmbeddingConfig
from litellm.types.llms.openai import AllEmbeddingInputValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

EMBEDDING_MODEL = "vercel_ai_gateway/text-embedding-3-small"


class ChatConfig(Protocol):
    @property
    def custom_llm_provider(self) -> str | None: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, Json]: ...

    def _get_openai_compatible_provider_info(
        self, api_base: str | None, api_key: str | None
    ) -> tuple[str | None, str | None]: ...

    def get_models(self, api_key: str | None = None, api_base: str | None = None) -> list[str]: ...


class EmbeddingConfig(Protocol):
    def validate_environment(  # noqa: PLR0913 - LiteLLM's signature
        self,
        headers: dict[str, Json],
        model: str,
        messages: list[object],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        *,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, Json]: ...

    def get_complete_url(  # noqa: PLR0913 - LiteLLM's signature
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        *,
        stream: bool | None = None,
    ) -> str: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, Json]: ...

    def transform_embedding_request(
        self,
        model: str,
        input: AllEmbeddingInputValues,  # noqa: A002 - LiteLLM's keyword name
        optional_params: dict[str, Json],
        headers: dict[str, Json],
    ) -> dict[str, Json]: ...


class _Response:
    def __init__(self, status_code: int, body: str) -> None:
        self.status_code = status_code
        self.body = body

    @property
    def text(self) -> str:
        return self.body

    def json(self) -> Json:
        return cast("Json", json.loads(self.body))


class _Client:
    def __init__(self, response: _Response) -> None:
        self.response = response
        self.url: str | None = None

    def get(self, url: str) -> _Response:
        self.url = url
        return self.response


def _object(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return cast("Object", value)


def _object_or_empty(value: Json, name: str) -> Object:
    if value is None:
        return {}
    return _object(value, name)


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _optional_text(value: Json, name: str) -> str | None:
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _string_map(value: Json, name: str) -> dict[str, str]:
    fields = _object_or_empty(value, name)
    result: dict[str, str] = {}
    for key, item in fields.items():
        if not isinstance(item, str):
            raise TypeError(f"{name} values must be strings")
        result[key] = item
    return result


def _string_set(value: Json) -> set[str]:
    if not isinstance(value, list):
        raise TypeError("model catalogs must be lists")
    result: set[str] = set()
    for item in value:
        if not isinstance(item, str):
            raise TypeError("model catalog values must be strings")
        result.add(item)
    return result


def _provider_step(step: Object, config: ChatConfig) -> Json:
    model = _text(step.get("model"), "model")
    api_base = _optional_text(step.get("api_base"), "api_base")
    api_key = _optional_text(step.get("api_key"), "api_key")
    environment = _string_map(step.get("environment"), "environment")
    with (
        patch.dict(os.environ, environment, clear=True),
        patch.object(
            litellm, "open_ai_chat_completion_models", _string_set(step.get("chat_models", []))
        ),
        patch.object(
            litellm, "open_ai_text_completion_models", _string_set(step.get("text_models", []))
        ),
    ):
        resolved_base, resolved_key = config._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            api_base, api_key
        )
        supported = config.get_supported_openai_params(model)
    return {
        "provider": config.custom_llm_provider,
        "api_base": resolved_base,
        "api_key": resolved_key,
        "supported_params": cast("Json", supported),
    }


def _map_step(step: Object, config: ChatConfig) -> Json:
    model = _text(step.get("model"), "model")
    requested = dict(_object_or_empty(step.get("non_default_params"), "non_default_params"))
    optional = dict(_object_or_empty(step.get("optional_params"), "optional_params"))
    with (
        patch.object(
            litellm, "open_ai_chat_completion_models", _string_set(step.get("chat_models", []))
        ),
        patch.object(
            litellm, "open_ai_text_completion_models", _string_set(step.get("text_models", []))
        ),
    ):
        mapped = config.map_openai_params(
            non_default_params=requested, optional_params=optional, model=model, drop_params=True
        )
    return {"optional_params": mapped, "remaining_params": requested}


def _headers_step(step: Object, config: EmbeddingConfig) -> Json:
    headers = dict(_object_or_empty(step.get("headers"), "headers"))
    api_key = _optional_text(step.get("api_key"), "api_key")
    api_base = _optional_text(step.get("api_base"), "api_base")
    environment = _string_map(step.get("environment"), "environment")
    with patch.dict(os.environ, environment, clear=True):
        merged = config.validate_environment(
            headers=headers,
            model=EMBEDDING_MODEL,
            messages=[],
            optional_params={},
            litellm_params={},
            api_key=api_key,
            api_base=api_base,
        )
        url = config.get_complete_url(
            api_base=api_base,
            api_key=api_key,
            model=EMBEDDING_MODEL,
            optional_params={},
            litellm_params={},
        )
    return {"headers": merged, "url": url}


def _request_step(step: Object, config: EmbeddingConfig) -> Json:
    model = _text(step.get("model"), "model")
    requested = _object_or_empty(step.get("non_default_params"), "non_default_params")
    optional = _object_or_empty(step.get("optional_params"), "optional_params")
    mapped = config.map_openai_params(
        non_default_params=dict(requested),
        optional_params=dict(optional),
        model=model,
        drop_params=True,
    )
    request = config.transform_embedding_request(
        model=model,
        input=cast("AllEmbeddingInputValues", step.get("input")),
        optional_params=mapped,
        headers={},
    )
    supported = config.get_supported_openai_params(model)
    return {"request": request, "params": mapped, "supported": cast("Json", supported)}


def _models_step(step: Object, config: ChatConfig) -> Json:
    api_base = _optional_text(step.get("api_base"), "api_base")
    environment = _string_map(step.get("environment"), "environment")
    status = step.get("status")
    if not isinstance(status, int) or isinstance(status, bool):
        raise TypeError("status must be an integer")
    client = _Client(_Response(status, _text(step.get("body"), "body")))
    with patch.dict(os.environ, environment, clear=True), patch.object(
        litellm, "module_level_client", client
    ):
        try:
            models = config.get_models(api_key=None, api_base=api_base)
            result: Json = {"models": cast("Json", models)}
        except Exception as error:  # noqa: BLE001 - the case compares the failure text as a value
            result = {"failure": str(error)}
    return {"url": client.url, "result": result}


def run(input_value: dict[str, Json]) -> Json:
    config = cast("ChatConfig", VercelAIGatewayConfig())
    embedding = cast("EmbeddingConfig", VercelAIGatewayEmbeddingConfig())
    steps = input_value.get("steps", [])
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    results: list[Json] = []
    for raw in steps:
        step = _object(raw, "step")
        kind = _text(step.get("kind"), "kind")
        if kind == "provider_info_and_supported_params":
            results.append(_provider_step(step, config))
        elif kind == "map_params":
            results.append(_map_step(step, config))
        elif kind == "embedding_headers_and_url":
            results.append(_headers_step(step, embedding))
        elif kind == "embedding_request_and_params":
            results.append(_request_step(step, embedding))
        elif kind == "models_url_and_list":
            results.append(_models_step(step, config))
        else:
            raise ValueError(f"unknown step kind {kind}")
    return results
