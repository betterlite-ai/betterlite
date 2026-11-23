import json
from typing import Final, Literal

from pydantic import JsonValue

from betterlite import _native

DEFAULT_BASE_TOKENS: Final = 85
DEFAULT_TILE_WIDTH: Final = 512
DEFAULT_TILE_HEIGHT: Final = 512


def resize_image_high_res(width: int, height: int) -> tuple[int, int]:
    request: dict[str, JsonValue] = {"width": width, "height": height}
    return _native.image_resize(json.dumps(request))


def calculate_tiles_needed(
    resized_width: int,
    resized_height: int,
    tile_width: int = DEFAULT_TILE_WIDTH,
    tile_height: int = DEFAULT_TILE_HEIGHT,
) -> int:
    request: dict[str, JsonValue] = {
        "resized_width": resized_width,
        "resized_height": resized_height,
    }
    if tile_width != DEFAULT_TILE_WIDTH or tile_height != DEFAULT_TILE_HEIGHT:
        request["tile_width"] = tile_width
        request["tile_height"] = tile_height
    return _native.image_tile_count(json.dumps(request))


# ast-grep-ignore: no-long-names
def high_detail_image_token_upper_bound(base_tokens: int = DEFAULT_BASE_TOKENS) -> int:
    request: dict[str, JsonValue] = {"base_tokens": base_tokens}
    return _native.image_high_detail_bound(json.dumps(request))


# ast-grep-ignore: no-one-line-functions
def get_image_type(image_data: bytes) -> str | None:
    return _native.image_type(image_data)


def get_image_dimensions(data: str) -> tuple[int, int]:
    request: dict[str, JsonValue] = {"data": data}
    return _native.image_dimensions(json.dumps(request))


def calculate_img_tokens(
    data: str,
    mode: Literal["low", "high", "auto"] = "auto",
    base_tokens: int = DEFAULT_BASE_TOKENS,
    use_default_image_token_count: bool = False,
) -> int:
    request: dict[str, JsonValue] = {
        "data": data,
        "mode": mode,
        "base_tokens": base_tokens,
        "use_default_image_token_count": use_default_image_token_count,
    }
    return _native.image_tokens(json.dumps(request))
