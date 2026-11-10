from collections.abc import Callable
from typing import TypeAlias, cast

import litellm
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import VertexGeminiConfig
from litellm.types.llms.vertex_ai import VERTEX_AI_PROVIDER_METADATA_FIELDS, Candidates
from litellm.types.utils import ModelResponse

litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Handler: TypeAlias = Callable[[Params], Json]
Extracted: TypeAlias = tuple[list[Json], list[Json], list[Json], list[Json]]


def _extract(candidate: Json) -> Extracted:
    extracted = VertexGeminiConfig._extract_candidate_metadata(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType, reportUnknownVariableType]
        cast(Candidates, candidate)
    )
    return cast("Extracted", extracted)


def _state(response: ModelResponse) -> Json:
    fields: dict[str, Json] = {}
    for name in VERTEX_AI_PROVIDER_METADATA_FIELDS:
        if hasattr(response, name):
            fields[name] = cast(Json, getattr(response, name))
    hidden = cast("dict[str, Json]", dict(response._hidden_params))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]
    return {"fields": fields, "hidden": hidden}


def scalar_vs_list(params: Params) -> Json:
    entries: list[Json] = []
    for candidate in cast("list[Json]", params["candidates"]):
        grounding, url_context, safety, citation = _extract(candidate)
        entries.append(
            {
                "grounding_metadata": grounding,
                "url_context_metadata": url_context,
                "safety_ratings": safety,
                "citation_metadata": citation,
            }
        )
    return {"candidates": entries}


def nested_safety(params: Params) -> Json:
    entries: list[Json] = []
    for candidate in cast("list[Json]", params["candidates"]):
        response = ModelResponse()
        VertexGeminiConfig._set_stream_metadata_on_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportArgumentType]
            response, *_extract(candidate)
        )
        entries.append(_state(response))
    return {"candidates": entries}


def _assembled(params: Params) -> Json:
    response = ModelResponse()
    chunks = cast("list[object]", cast("list[Json]", params["chunks"]))
    VertexGeminiConfig().apply_assembled_streaming_response_metadata(response, chunks)
    return _state(response)


def _annotations(params: Params) -> Json:
    annotations = VertexGeminiConfig._convert_grounding_metadata_to_annotations(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        cast("list[dict[str, Json]]", cast("list[Json]", params["grounding_metadata"])),
        cast("str | None", params.get("content_text")),
    )
    return {"annotations": cast(Json, annotations)}


HANDLERS: dict[str, Handler] = {
    "scalar_vs_list": scalar_vs_list,
    "nested_safety": nested_safety,
    "hidden_fallback": _assembled,
    "empty_visible_shadow": _assembled,
    "duplicate_chunks": _assembled,
    "first_nonweb_index": _annotations,
    "citation_empty_title": _annotations,
}


def run(input_value: dict[str, Json]) -> Json:
    handler = HANDLERS.get(cast(str, input_value["call"]))
    if handler is None:
        raise ValueError(f"unknown operation: {input_value['call']}")
    return handler(input_value)
