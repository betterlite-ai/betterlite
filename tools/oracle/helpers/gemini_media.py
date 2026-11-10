from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.vertex_ai.gemini import transformation
from litellm.llms.vertex_ai.gemini.transformation import (
    _apply_gemini_mime_type_aliases,  # pyright: ignore[reportPrivateUsage]
    _convert_detail_to_media_resolution_enum,  # pyright: ignore[reportPrivateUsage]
    _explicit_mime_type,  # pyright: ignore[reportPrivateUsage]
    _extract_max_media_resolution_from_messages,  # pyright: ignore[reportPrivateUsage]
    _process_gemini_media,  # pyright: ignore[reportPrivateUsage]
    check_if_part_exists_in_parts,
)
from litellm.types.llms.vertex_ai import PartType

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Handler: TypeAlias = Callable[[Params], Json]

litellm.suppress_debug_info = True


def _blocked_metadata(**_kwargs: object) -> str | None:
    raise RuntimeError("gemini_media cases must not read GCS metadata")


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _list(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def _string(params: Params, name: str) -> str:
    value = params.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_string(params: Params, name: str) -> str | None:
    value = params.get(name)
    return value if isinstance(value, str) else None


def _mime_alias(params: Params) -> Json:
    values = [cast(str, item) for item in _list(params["values"], "values")]
    return {"values": [_apply_gemini_mime_type_aliases(value) for value in values]}


def _media_parts(params: Params) -> Json:
    return {"parts": [_media_part(case) for case in _list(params["cases"], "cases")]}


def _media_part(case: Json) -> Json:
    fields = _object(case, "case")
    hints_value = fields.get("hints")
    hints = {} if hints_value is None else _object(hints_value, "hints")
    metadata_value = fields.get("video_metadata")
    metadata = (
        None if metadata_value is None else _object(metadata_value, "video_metadata")
    )
    with patch.object(transformation, "_get_gcs_object_content_type", _blocked_metadata):
        part = _process_gemini_media(
            image_url=_string(fields, "url"),
            format=_explicit_mime_type(hints),
            media_resolution_enum=_convert_detail_to_media_resolution_enum(
                _optional_string(fields, "detail")
            ),
            model=_optional_string(fields, "model"),
            video_metadata=metadata,
        )
    return cast(Json, part)


def _resolution_maximum(params: Params) -> Json:
    messages = _list(params["messages"], "messages")
    maximum = _extract_max_media_resolution_from_messages(
        cast("list[AllMessageValues]", messages)
    )
    return {"maximum": maximum}


def _part_key_equivalence(params: Params) -> Json:
    results: list[Json] = []
    for case in _list(params["cases"], "cases"):
        fields = _object(case, "case")
        parts = [
            cast(PartType, _object(item, "part"))
            for item in _list(fields["parts"], "parts")
        ]
        part = cast(PartType, _object(fields["part"], "part"))
        excluded: list[str] = []
        excluded_value = fields.get("excluded_keys")
        if excluded_value is not None:
            excluded = [
                cast(str, item)
                for item in _list(excluded_value, "excluded_keys")
            ]
        results.append(check_if_part_exists_in_parts(parts, part, excluded_keys=excluded))
    return {"matches": results}


HANDLERS: dict[str, Handler] = {
    "mime_alias": _mime_alias,
    "explicit_gcs_format": _media_parts,
    "files_uri_without_mime": _media_parts,
    "https_passthrough": _media_parts,
    "video_offsets": _media_parts,
    "resolution_maximum": _resolution_maximum,
    "part_key_equivalence": _part_key_equivalence,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = _string(input_value, "call")
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    return handler(input_value)
