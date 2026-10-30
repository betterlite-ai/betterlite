from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates.common_utils import (
    DEFS_MAX_INLINED_BYTES,
    _estimate_json_bytes,  # pyright: ignore[reportPrivateUsage]
    _has_legacy_defs,  # pyright: ignore[reportPrivateUsage]
    sanitize_input_schema_for_anthropic,  # pyright: ignore[reportUnknownVariableType]
    unpack_defs,  # pyright: ignore[reportUnknownVariableType]
    unpack_legacy_defs,  # pyright: ignore[reportUnknownVariableType]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _estimate_json_bytes_mode(input_value: dict[str, Json]) -> Json:
    return _estimate_json_bytes(input_value["value"])


def _unpack_defs_mode(input_value: dict[str, Json]) -> Json:
    schema = cast(dict[str, Json], input_value["schema"])
    defs = cast(dict[str, Json], input_value["defs"])
    max_inlined_bytes = cast(int | None, input_value.get("max_inlined_bytes"))
    unpack_defs(schema, defs, max_inlined_bytes=max_inlined_bytes)
    return cast(Json, schema)


def _has_legacy_defs_mode(input_value: dict[str, Json]) -> Json:
    return _has_legacy_defs(input_value["schema"])


def _unpack_legacy_defs_mode(input_value: dict[str, Json]) -> Json:
    schema = cast(dict[str, Json], input_value["schema"])
    max_inlined_bytes = cast(int | None, input_value.get("max_inlined_bytes"))
    return cast(
        Json,
        unpack_legacy_defs(
            schema,
            max_inlined_bytes=max_inlined_bytes
            if max_inlined_bytes is not None
            else DEFS_MAX_INLINED_BYTES,
        ),
    )


def _sanitize_input_schema_for_anthropic_mode(input_value: dict[str, Json]) -> Json:
    input_schema = cast(dict[str, Json], input_value["input_schema"])
    return cast(Json, dict(sanitize_input_schema_for_anthropic(input_schema)))


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "estimate_json_bytes": _estimate_json_bytes_mode,
    "unpack_defs": _unpack_defs_mode,
    "has_legacy_defs": _has_legacy_defs_mode,
    "unpack_legacy_defs": _unpack_legacy_defs_mode,
    "sanitize_input_schema_for_anthropic": _sanitize_input_schema_for_anthropic_mode,
}


def run(input_value: dict[str, Json]) -> Json:
    mode = cast(str, input_value["mode"])
    if mode not in HANDLERS:
        raise ValueError(f"unsupported mode {mode}")
    return HANDLERS[mode](input_value)
