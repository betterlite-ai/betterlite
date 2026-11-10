import os
from typing import TYPE_CHECKING, Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm.llms.huggingface.embedding.handler as embedding_handler
from litellm.litellm_core_utils.default_encoding import encoding as default_encoding
from litellm.llms.huggingface.common_utils import HuggingFaceError
from litellm.llms.huggingface.embedding.transformation import HuggingFaceEmbeddingConfig
from litellm.types.utils import EmbeddingResponse

if TYPE_CHECKING:
    from collections.abc import Callable

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class Encoding(Protocol):
    def encode_ordinary(self, text: str) -> list[int]: ...


class EmbeddingHandler(Protocol):
    def _transform_input(
        self,
        embedding_input: list[str],
        model: str,
        call_type: str,
        optional_params: Object,
        embed_url: str,
    ) -> Object: ...

    def _transform_input_on_pipeline_tag(
        self, embedding_input: list[str], pipeline_tag: str | None
    ) -> Object: ...

    def _process_optional_params(self, data: Object, optional_params: Object) -> Object: ...

    def _process_embedding_response(
        self,
        embeddings: Json,
        model_response: EmbeddingResponse,
        model: str,
        embedding_input: list[str],
        encoding: Encoding,
    ) -> EmbeddingResponse: ...


HANDLER = cast(EmbeddingHandler, embedding_handler.HuggingFaceEmbedding())
VALIDATE = cast(
    "Callable[..., dict[str, str]]",
    HuggingFaceEmbeddingConfig().validate_environment,
)
ENCODING = cast(Encoding, default_encoding)
TRANSFORM_INPUT = cast(
    "Callable[..., Object]",
    HANDLER._transform_input,  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
)
TRANSFORM_ON_TAG = cast(
    "Callable[..., Object]",
    HANDLER._transform_input_on_pipeline_tag,  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
)
PROCESS_OPTIONAL_PARAMS = cast(
    "Callable[..., Object]",
    HANDLER._process_optional_params,  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
)
PROCESS_RESPONSE = cast(
    "Callable[..., EmbeddingResponse]",
    HANDLER._process_embedding_response,  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
)


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def text_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(value: Json, name: str) -> str | None:
    return None if value is None else text_value(value, name)


def string_map(value: Json, name: str) -> dict[str, str]:
    result: dict[str, str] = {}
    for key, item in object_value(value, name).items():
        result[key] = text_value(item, key)
    return result


def string_list(value: Json, name: str) -> list[str]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [text_value(item, name) for item in value]


def apply_environment(value: Json) -> None:
    for name in ("HF_API_BASE", "HUGGINGFACE_API_BASE"):
        os.environ.pop(name, None)
    for name, item in object_value(value, "env").items():
        os.environ[name] = text_value(item, name)


def embedding_url(model: str, api_base: str | None, task: str | None) -> str:
    if model.startswith(("http://", "https://")):
        return model
    if api_base:
        return api_base
    if "HF_API_BASE" in os.environ:
        return os.getenv("HF_API_BASE", "")
    if "HUGGINGFACE_API_BASE" in os.environ:
        return os.getenv("HUGGINGFACE_API_BASE", "")
    return f"https://router.huggingface.co/hf-inference/pipeline/{task}/{model}"


class Hub:
    def __init__(self) -> None:
        self.pipeline_tag: str | None = None
        self.requests: list[str] = []

    def client(self, **_kwargs: object) -> "HubClient":
        return HubClient(self)


class HubClient:
    def __init__(self, hub: Hub) -> None:
        self.hub = hub

    def get(self, url: str, **_kwargs: object) -> httpx.Response:
        self.hub.requests.append(url)
        body: Object = {}
        if self.hub.pipeline_tag is not None:
            body["pipeline_tag"] = self.hub.pipeline_tag
        return httpx.Response(200, json=body)


def hub_lookups(hub: Hub) -> list[Json]:
    prefix = f"{embedding_handler.HF_HUB_URL}/api/models/"
    return [url.removeprefix(prefix) for url in hub.requests]


def prepare_step(step: Object, hub: Hub) -> Json:
    model = text_value(step.get("model"), "model")
    inputs = string_list(step.get("input"), "input")
    optional_params = object_value(step.get("optional_params", {}), "optional_params")
    headers = string_map(step.get("headers", {}), "headers")
    api_key = optional_text(step.get("api_key"), "api_key")
    api_base = optional_text(step.get("api_base"), "api_base")
    apply_environment(step.get("env", {}))
    with patch.object(embedding_handler, "HTTPHandler", hub.client):
        validated = VALIDATE(headers, model, [], optional_params, {}, api_key=api_key)
        task = embedding_handler.get_hf_task_embedding_for_model(
            model=model,
            task_type=cast("str | None", optional_params.get("input_type")),
            api_base=embedding_handler.HF_HUB_URL,
        )
        url = embedding_url(model, api_base, task)
        body = TRANSFORM_INPUT(inputs, model, "sync", dict(optional_params), url)
    return {
        "url": url,
        "headers": cast(Json, validated),
        "body": body,
        "lookups": hub_lookups(hub),
    }


def transform_step(step: Object, hub: Hub) -> Json:
    model = text_value(step.get("model"), "model")
    inputs = string_list(step.get("input"), "input")
    optional_params = object_value(step.get("optional_params", {}), "optional_params")
    with patch.object(embedding_handler, "HTTPHandler", hub.client):
        body = TRANSFORM_INPUT(inputs, model, "sync", dict(optional_params), "")
    return {"body": body, "lookups": hub_lookups(hub)}


def pipeline_tag_step(step: Object) -> Json:
    inputs = string_list(step.get("input"), "input")
    pipeline_tag = optional_text(step.get("pipeline_tag"), "pipeline_tag")
    return TRANSFORM_ON_TAG(inputs, pipeline_tag)


def optional_params_step(step: Object) -> Json:
    data = object_value(step.get("data"), "data")
    optional_params = object_value(step.get("optional_params", {}), "optional_params")
    ordered = step.get("ordered_params", [])
    if not isinstance(ordered, list):
        raise TypeError("ordered_params must be a list")
    for pair in ordered:
        match pair:
            case [str() as name, item]:
                optional_params[name] = item
            case _:
                raise TypeError("ordered_params entries must be [name, value] pairs")
    return PROCESS_OPTIONAL_PARAMS(data, optional_params)


def response_step(step: Object) -> Json:
    model = text_value(step.get("model"), "model")
    inputs = string_list(step.get("input"), "input")
    embeddings = step.get("embeddings")
    if isinstance(embeddings, dict) and "error" in embeddings:
        raise HuggingFaceError(status_code=500, message=embeddings["error"])
    response = PROCESS_RESPONSE(
        cast("Object | list[Json]", embeddings),
        EmbeddingResponse(),
        model,
        inputs,
        ENCODING,
    )
    usage = response.usage
    if usage is None:
        raise TypeError("the embedding response has no usage")
    return {
        "object": response.object,
        "data": cast(Json, response.data),
        "model": response.model,
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        },
    }


def validate_step(step: Object) -> Json:
    headers = string_map(step.get("headers", {}), "headers")
    api_key = optional_text(step.get("api_key"), "api_key")
    validated = VALIDATE(headers, "", [], {}, {}, api_key=api_key)
    return cast(Json, validated)


def url_step(step: Object) -> Json:
    model = text_value(step.get("model"), "model")
    api_base = optional_text(step.get("api_base"), "api_base")
    task = optional_text(step.get("task"), "task")
    apply_environment(step.get("env", {}))
    return embedding_url(model, api_base, task)


def run_step(step: Object, hub: Hub) -> Json:
    kind = text_value(step.get("kind"), "kind")
    if kind == "prepare":
        outcome = prepare_step(step, hub)
    elif kind == "transform_input":
        outcome = transform_step(step, hub)
    elif kind == "pipeline_tag":
        outcome = pipeline_tag_step(step)
    elif kind == "optional_params":
        outcome = optional_params_step(step)
    elif kind == "response":
        outcome = response_step(step)
    elif kind == "validate_environment":
        outcome = validate_step(step)
    elif kind == "url":
        outcome = url_step(step)
    else:
        raise ValueError(f"unknown step kind {kind}")
    return outcome


def run(value: dict[str, Json]) -> Json:
    steps = value.get("steps")
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    outcomes: list[Json] = []
    for step in steps:
        entry = object_value(step, "step")
        hub = Hub()
        hub.pipeline_tag = optional_text(entry.get("pipeline_tag"), "pipeline_tag")
        try:
            outcomes.append({"ok": True, "value": run_step(entry, hub)})
        except Exception as error:  # noqa: BLE001 - the outcome carries the status a case compares
            carried: object = getattr(error, "status_code", None)
            status = carried if isinstance(carried, int) and not isinstance(carried, bool) else None
            outcomes.append({"ok": False, "status": status})
    return outcomes
