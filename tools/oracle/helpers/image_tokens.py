import base64
from collections.abc import Callable
from typing import Literal, TypeAlias, cast
from unittest.mock import patch

from litellm.litellm_core_utils import token_counter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Mode: TypeAlias = Literal["low", "high", "auto"]

calculate_img_tokens = cast(Callable[..., int], token_counter.calculate_img_tokens)
calculate_tiles_needed = cast(Callable[..., int], token_counter.calculate_tiles_needed)
resize_image_high_res = cast(
    Callable[..., tuple[int, int]], token_counter.resize_image_high_res
)
high_detail_image_token_upper_bound = cast(
    Callable[..., int], token_counter.high_detail_image_token_upper_bound
)
get_image_type = cast(Callable[..., str | None], token_counter.get_image_type)
get_image_dimensions = cast(
    Callable[..., tuple[int, int]], token_counter.get_image_dimensions
)

DEFAULT_SETTINGS: dict[str, int] = {
    "default_width": 300,
    "default_height": 300,
    "default_token_count": 250,
    "max_short_side": 768,
    "max_long_side": 2000,
    "tile_width": 512,
    "tile_height": 512,
}


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _integer(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def _boolean(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def _mode(value: Json) -> Mode:
    if value is None:
        return "auto"
    if isinstance(value, str):
        match value:
            case "low" | "high" | "auto":
                return value
            case _:
                pass
    raise ValueError("mode must be 'low', 'high' or 'auto'")


def _settings(input_value: dict[str, Json]) -> dict[str, int]:
    raw = input_value.get("settings")
    if raw is None:
        return dict(DEFAULT_SETTINGS)
    if not isinstance(raw, dict):
        raise ValueError("settings must be an object")
    settings = dict(DEFAULT_SETTINGS)
    for name, value in raw.items():
        if name not in settings:
            raise ValueError(f"unknown setting {name}")
        settings[name] = _integer(value, name)
    return settings


def _calls(input_value: dict[str, Json]) -> list[dict[str, Json]]:
    value = input_value.get("calls")
    if not isinstance(value, list):
        raise ValueError("calls must be a list")
    calls: list[dict[str, Json]] = []
    for call in value:
        if not isinstance(call, dict):
            raise ValueError("each call must be an object")
        calls.append(call)
    return calls


def _decoded(data: Json) -> bytes:
    text = _string(data, "data")
    _header, encoded = text.split(",", 1)
    return base64.b64decode(encoded)


def _count_options(call: dict[str, Json]) -> dict[str, Json]:
    options: dict[str, Json] = {}
    mode = call.get("mode")
    if mode is not None:
        options["mode"] = _mode(mode)
    base_tokens = call.get("base_tokens")
    if base_tokens is not None:
        options["base_tokens"] = _integer(base_tokens, "base_tokens")
    use_default = call.get("use_default_image_token_count")
    if use_default is not None:
        options["use_default_image_token_count"] = _boolean(
            use_default, "use_default_image_token_count"
        )
    return options


def _upper_bound(call: dict[str, Json]) -> Json:
    base_tokens = call.get("base_tokens")
    if base_tokens is None:
        return high_detail_image_token_upper_bound()
    return high_detail_image_token_upper_bound(_integer(base_tokens, "base_tokens"))


def _call(call: dict[str, Json]) -> Json:
    function = call.get("function")
    match function:
        case "calculate_img_tokens":
            return calculate_img_tokens(
                data=_string(call.get("data"), "data"), **_count_options(call)
            )
        case "resize_image_high_res":
            width, height = resize_image_high_res(
                _integer(call.get("width"), "width"),
                _integer(call.get("height"), "height"),
            )
            return [width, height]
        case "calculate_tiles_needed":
            return calculate_tiles_needed(
                _integer(call.get("resized_width"), "resized_width"),
                _integer(call.get("resized_height"), "resized_height"),
            )
        case "high_detail_image_token_upper_bound":
            return _upper_bound(call)
        case "get_image_type":
            return get_image_type(_decoded(call.get("data")))
        case "get_image_dimensions":
            width, height = get_image_dimensions(_string(call.get("data"), "data"))
            return [width, height]
        case other:
            raise ValueError(f"unknown image_tokens function: {other!r}")


def run(input_value: dict[str, Json]) -> Json:
    settings = _settings(input_value)
    calls = _calls(input_value)
    with (
        patch.object(token_counter, "DEFAULT_IMAGE_WIDTH", settings["default_width"]),
        patch.object(token_counter, "DEFAULT_IMAGE_HEIGHT", settings["default_height"]),
        patch.object(token_counter, "DEFAULT_IMAGE_TOKEN_COUNT", settings["default_token_count"]),
        patch.object(
            token_counter, "MAX_SHORT_SIDE_FOR_IMAGE_HIGH_RES", settings["max_short_side"]
        ),
        patch.object(
            token_counter, "MAX_LONG_SIDE_FOR_IMAGE_HIGH_RES", settings["max_long_side"]
        ),
        patch.object(
            calculate_tiles_needed,
            "__defaults__",
            (settings["tile_width"], settings["tile_height"]),
        ),
    ):
        return [_call(call) for call in calls]
