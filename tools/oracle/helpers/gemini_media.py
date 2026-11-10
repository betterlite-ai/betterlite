from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.exceptions import BadRequestError
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


def _mime_alias(params: Params) -> Json:
    values = cast("list[str]", params["values"])
    return {"values": [_apply_gemini_mime_type_aliases(value) for value in values]}


def _media_parts(params: Params) -> Json:
    return {"parts": [_media_part(case) for case in cast("list[Json]", params["cases"])]}


def _media_part(case: Json) -> Json:
    fields = cast("dict[str, Json]", case)
    hints = cast("dict[str, Json]", fields.get("hints") or {})
    try:
        with patch.object(transformation, "_get_gcs_object_content_type", _blocked_metadata):
            part = _process_gemini_media(
                image_url=cast(str, fields["url"]),
                format=_explicit_mime_type(hints),
                media_resolution_enum=_convert_detail_to_media_resolution_enum(
                    cast("str | None", fields.get("detail"))
                ),
                model=cast("str | None", fields.get("model")),
                video_metadata=cast("dict[str, Json] | None", fields.get("video_metadata")),
            )
    except BadRequestError as error:
        status = error.status_code
        return {"error": {"status": status, "type": None, "param": None, "code": str(status)}}
    return cast(Json, part)


def _resolution_maximum(params: Params) -> Json:
    messages = cast("list[AllMessageValues]", params["messages"])
    return {"maximum": _extract_max_media_resolution_from_messages(messages)}


def _part_key_equivalence(params: Params) -> Json:
    results: list[Json] = []
    for case in cast("list[Json]", params["cases"]):
        fields = cast("dict[str, Json]", case)
        results.append(
            check_if_part_exists_in_parts(
                cast("list[PartType]", fields["parts"]),
                cast(PartType, fields["part"]),
                excluded_keys=cast("list[str]", fields.get("excluded_keys") or []),
            )
        )
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
    handler = HANDLERS.get(cast(str, input_value["call"]))
    if handler is None:
        raise ValueError(f"unknown operation: {input_value['call']}")
    return handler(input_value)
