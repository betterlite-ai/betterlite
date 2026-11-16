import base64
from collections.abc import Callable
from typing import Final, Literal, NotRequired, Protocol, TypedDict, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.litellm_core_utils import token_counter
from litellm.litellm_core_utils.token_counter import (
    _count_content_list,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/token_counter.py#L869-L951
    _get_count_function,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/token_counter.py#L608-L612
)
from pydantic import JsonValue

litellm.suppress_debug_info = True

DEFAULT_MODEL: Final = "gpt-3.5-turbo"
DEFAULT_SETTINGS: Final[dict[str, int]] = {
    "default_width": 300,
    "default_height": 300,
    "default_token_count": 250,
    "max_short_side": 768,
    "max_long_side": 2000,
    "tile_width": 512,
    "tile_height": 512,
}


class CountContentList(Protocol):
    def __call__(
        self,
        *,
        count_function: Callable[[str], int],
        content_list: list[JsonValue],
        use_default_image_token_count: bool,
        default_token_count: int | None,
    ) -> int: ...


count_content = cast(CountContentList, _count_content_list)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/token_counter.py#L869-L951
calculate_tiles_needed = cast("Callable[[int, int, int, int], int]", token_counter.calculate_tiles_needed)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/token_counter.py#L173-L178


class Call(TypedDict):
    function: Literal["count_content"]
    content: list[JsonValue]
    use_default_image_token_count: NotRequired[bool]
    default_token_count: NotRequired[int | None]


class Input(TypedDict):
    settings: NotRequired[dict[str, int]]
    urls: NotRequired[dict[str, str]]
    calls: list[Call]


class Answers:
    def __init__(self, urls: dict[str, str]) -> None:
        self.urls: dict[str, str] = urls

    def __call__(self, _client: object, url: str, **_options: object) -> httpx.Response:
        data = self.urls.get(url)
        if data is None:
            raise httpx.ConnectError("the helper has no bytes for this URL")
        _header, encoded = data.split(",", 1)
        return httpx.Response(200, content=base64.b64decode(encoded))


def no_client(params: dict[str, str] | None = None) -> None:
    _ = params


def run(input_value: Input) -> list[int]:
    settings = DEFAULT_SETTINGS.copy()
    given = input_value.get("settings")
    if given is not None:
        unknown = set(given) - set(DEFAULT_SETTINGS)
        if unknown:
            raise ValueError(f"unknown image setting {min(unknown)}")
        settings.update(given)
    urls = input_value.get("urls")
    answers = Answers({} if urls is None else urls)
    with (
        patch.object(token_counter, "DEFAULT_IMAGE_WIDTH", settings["default_width"]),
        patch.object(token_counter, "DEFAULT_IMAGE_HEIGHT", settings["default_height"]),
        patch.object(token_counter, "DEFAULT_IMAGE_TOKEN_COUNT", settings["default_token_count"]),
        patch.object(
            token_counter, "MAX_SHORT_SIDE_FOR_IMAGE_HIGH_RES", settings["max_short_side"]
        ),
        patch.object(token_counter, "MAX_LONG_SIDE_FOR_IMAGE_HIGH_RES", settings["max_long_side"]),
        patch.object(
            calculate_tiles_needed,
            "__defaults__",
            (settings["tile_width"], settings["tile_height"]),
        ),
        patch.object(token_counter, "safe_get", answers),
        patch.object(token_counter, "_get_httpx_client", no_client),
    ):
        return [
            count_content(
                count_function=_get_count_function(DEFAULT_MODEL),
                content_list=call["content"],
                use_default_image_token_count=call.get("use_default_image_token_count", False),
                default_token_count=call.get("default_token_count"),
            )
            for call in input_value["calls"]
        ]
