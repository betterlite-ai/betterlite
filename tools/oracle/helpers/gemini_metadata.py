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
Extractor: TypeAlias = Callable[[Candidates], Extracted]
Writer: TypeAlias = Callable[[ModelResponse, list[Json], list[Json], list[Json], list[Json]], None]


def _extract(candidate: Json) -> Extracted:
    extract = cast(Extractor, VertexGeminiConfig._extract_candidate_metadata)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]
    return extract(cast(Candidates, candidate))


def _write(response: ModelResponse, values: Extracted) -> None:
    write = cast(Writer, VertexGeminiConfig._set_stream_metadata_on_response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]
    grounding, url_context, safety, citation = values
    write(response, grounding, url_context, safety, citation)


def _visible(response: ModelResponse) -> dict[str, Json]:
    found: dict[str, Json] = {}
    for name in VERTEX_AI_PROVIDER_METADATA_FIELDS:
        if hasattr(response, name):
            found[name] = cast(Json, getattr(response, name))
    return found


def _hidden(response: ModelResponse) -> dict[str, Json]:
    return cast("dict[str, Json]", dict(response._hidden_params))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]


def _extracted(candidate: Json) -> Json:
    grounding, url_context, safety, citation = _extract(candidate)
    return {
        "grounding_metadata": grounding,
        "url_context_metadata": url_context,
        "safety_ratings": safety,
        "citation_metadata": citation,
    }


def _applied(candidate: Json) -> Json:
    response = ModelResponse()
    _write(response, _extract(candidate))
    return {"fields": _visible(response), "hidden": _hidden(response)}


def scalar_vs_list(params: Params) -> Json:
    return {
        "candidates": [
            _extracted(candidate) for candidate in cast("list[Json]", params["candidates"])
        ]
    }


def nested_safety(params: Params) -> Json:
    return {
        "candidates": [
            _applied(candidate) for candidate in cast("list[Json]", params["candidates"])
        ]
    }


def _assembled(params: Params) -> Json:
    response = ModelResponse()
    chunks = cast("list[object]", cast("list[Json]", params["chunks"]))
    VertexGeminiConfig().apply_assembled_streaming_response_metadata(response, chunks)
    return {"fields": _visible(response), "hidden": _hidden(response)}


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
