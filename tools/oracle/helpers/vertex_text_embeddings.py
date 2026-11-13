from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.llms.vertex_ai.vertex_embeddings.transformation import VertexAITextEmbeddingConfig
from litellm.types.utils import EmbeddingResponse

if TYPE_CHECKING:
    from collections.abc import Callable

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Input: TypeAlias = str | list[str]

CONFIG = VertexAITextEmbeddingConfig()

MAP = cast(
    "Callable[[Object, Object, Object], tuple[Object, Object]]",
    CONFIG.map_openai_params,
)
AUTH = cast("Callable[[Object, Object], Object]", CONFIG.map_special_auth_params)
REQUEST = cast(
    "Callable[[Input, Object, str, Object | None], Object]",
    CONFIG.transform_openai_request_to_vertex_embedding_request,
)
RESPONSE = cast(
    "Callable[[Json, str, EmbeddingResponse], EmbeddingResponse]",
    CONFIG.transform_vertex_response_to_openai,
)


def object_of(value: Json | None, name: str) -> Object:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def object_field(value: Object, name: str) -> Object:
    present = value.get(name)
    if present is None:
        return {}
    return object_of(present, name)


def string_field(value: Object, name: str) -> str:
    present = value.get(name)
    if not isinstance(present, str):
        raise ValueError(f"{name} must be a string")
    return present


def list_field(value: Object, name: str) -> list[Json]:
    present = value.get(name)
    if present is None:
        return []
    if not isinstance(present, list):
        raise ValueError(f"{name} must be a list")
    return present


def mapping_entry(call: Json) -> Json:
    entry = object_of(call, "mapping")
    options = object_field(entry, "options")
    optional = object_field(entry, "optional")
    mapped_optional, mapped_kwargs = MAP(options, optional, object_field(entry, "kwargs"))
    return cast(
        Json,
        {
            "optional": cast(Json, mapped_optional),
            "kwargs": cast(Json, mapped_kwargs),
            "auth": cast(Json, AUTH(options, optional)),
        },
    )


def request_entry(call: Json) -> Json:
    entry = object_of(call, "request")
    model = string_field(entry, "model")
    optional = object_field(entry, "optional")
    params = entry.get("litellm_params")
    try:
        request = REQUEST(
            cast(Input, entry.get("input")),
            optional,
            model,
            None if params is None else object_of(params, "litellm_params"),
        )
        return cast(Json, cast(object, request))
    except Exception:  # noqa: BLE001 - a rejected transform is compared as a marker
        return cast(Json, {"error": True})


def reply_entry(call: Json) -> Json:
    entry = object_of(call, "reply")
    try:
        answer = RESPONSE(entry.get("response"), string_field(entry, "model"), EmbeddingResponse())
        usage = answer.usage
        if usage is None:
            raise ValueError("the answer carries no usage")
        return cast(
            Json,
            {
                "model": answer.model,
                "object": answer.object,
                "data": cast(Json, cast(object, answer.data)),
                "usage": {
                    "prompt_tokens": usage.prompt_tokens,
                    "completion_tokens": usage.completion_tokens,
                    "total_tokens": usage.total_tokens,
                },
            },
        )
    except Exception:  # noqa: BLE001 - a rejected answer is compared as a marker
        return cast(Json, {"error": True})


def run(value: dict[str, Json]) -> Json:
    return {
        "mappings": [mapping_entry(call) for call in list_field(value, "mappings")],
        "requests": [request_entry(call) for call in list_field(value, "requests")],
        "replies": [reply_entry(call) for call in list_field(value, "replies")],
    }
