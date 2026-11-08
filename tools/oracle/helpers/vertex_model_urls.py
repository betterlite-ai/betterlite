from collections.abc import Callable
from typing import Literal, TypeAlias, cast

from litellm.llms.vertex_ai.common_utils import (
    _get_embedding_url,  # pyright: ignore[reportPrivateUsage]
    _get_gemini_url,  # pyright: ignore[reportPrivateUsage]
    _get_vertex_url,  # pyright: ignore[reportPrivateUsage]
    get_vertex_ai_fine_tuned_endpoint_id,
    get_vertex_base_model_name,
    get_vertex_base_url,
)
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import VertexGeminiConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Mode: TypeAlias = Literal[
    "chat", "embedding", "batch_embedding", "image_generation", "count_tokens"
]
Version: TypeAlias = Literal["v1", "v1beta1"]
Handler: TypeAlias = Callable[[Params], Json]


def object_of(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_of(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_of(params: Params, name: str) -> str:
    value = params.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(params: Params, name: str) -> str | None:
    value = params.get(name)
    return value if isinstance(value, str) else None


def optional_bool(params: Params, name: str) -> bool | None:
    value = params.get(name)
    return value if isinstance(value, bool) else None


def url_of(pair: tuple[str, str]) -> Json:
    return {"url": pair[0], "endpoint": pair[1]}


def base_url(params: Params) -> Json:
    return {
        "urls": [
            base_url_of(item) for item in list_of(params["locations"], "locations")
        ]
    }


def base_url_of(item: Json) -> Json:
    try:
        return {"url": get_vertex_base_url(item if isinstance(item, str) else None)}
    except ValueError:
        return {"error": True}


def vertex_url(params: Params) -> Json:
    urls: list[Json] = []
    for item in list_of(params["cases"], "cases"):
        fields = object_of(item, "case")
        try:
            pair = _get_vertex_url(
                cast(Mode, string_of(fields, "mode")),
                string_of(fields, "model"),
                optional_bool(fields, "stream"),
                optional_string(fields, "project"),
                optional_string(fields, "location"),
                cast(Version, string_of(fields, "api_version")),
            )
        except ValueError:
            urls.append({"error": True})
            continue
        urls.append(url_of(pair))
    return {"urls": urls}


def embedding_url(params: Params) -> Json:
    urls: list[Json] = []
    for item in list_of(params["cases"], "cases"):
        fields = object_of(item, "case")
        try:
            pair = _get_embedding_url(
                string_of(fields, "model"),
                optional_string(fields, "project"),
                optional_string(fields, "location"),
                cast(Version, string_of(fields, "api_version")),
            )
        except ValueError:
            urls.append({"error": True})
            continue
        urls.append(url_of(pair))
    return {"urls": urls}


def gemini_url(params: Params) -> Json:
    urls: list[Json] = []
    for item in list_of(params["cases"], "cases"):
        fields = object_of(item, "case")
        try:
            pair = _get_gemini_url(
                cast(Mode, string_of(fields, "mode")),
                string_of(fields, "model"),
                optional_bool(fields, "stream"),
            )
        except ValueError:
            urls.append({"error": True})
            continue
        urls.append(url_of(pair))
    return {"urls": urls}


def model_predicates(params: Params) -> Json:
    models: list[Json] = []
    for item in list_of(params["models"], "models"):
        model = cast(str, item)
        models.append(
            {
                "base_name": get_vertex_base_model_name(model),
                "fine_tuned_id": get_vertex_ai_fine_tuned_endpoint_id(model),
                "url_model": VertexGeminiConfig.get_model_for_vertex_ai_url(model),
                "newer": VertexGeminiConfig._is_gemini_3_or_newer(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    model
                ),
                "forward": VertexGeminiConfig._forward_gemini_function_call_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    model
                ),
            }
        )
    return {"models": models}


def unsupported_modes(params: Params) -> Json:
    modes: list[Json] = []
    for item in list_of(params["modes"], "modes"):
        mode = cast(str, item)
        if mode == "batch_embedding":
            modes.append({"mode": mode, "error": batch_embedding_rejected()})
        else:
            modes.append({"mode": mode, "error": image_generation_rejected()})
    return {"modes": modes}


def batch_embedding_rejected() -> bool:
    try:
        _ = _get_vertex_url(
            "batch_embedding", "gemini-embedding-2", None, "proj", "us-central1", "v1"
        )
    except ValueError:
        return True
    return False


def image_generation_rejected() -> bool:
    try:
        _ = _get_gemini_url("image_generation", "gemini-2.5-flash", None)
    except ValueError:
        return True
    return False


HANDLERS: dict[str, Handler] = {
    "base_url": base_url,
    "vertex_url": vertex_url,
    "embedding_url": embedding_url,
    "gemini_url": gemini_url,
    "model_predicates": model_predicates,
    "unsupported_modes": unsupported_modes,
}


def run(input_value: dict[str, Json]) -> Json:
    if len(input_value) != 1:
        raise ValueError("vertex_model_urls takes exactly one operation")
    operation, payload = next(iter(input_value.items()))
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    return handler(object_of(payload, operation))
