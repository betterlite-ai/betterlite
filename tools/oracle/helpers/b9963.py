import base64
from collections.abc import Callable
from typing import Final, Literal, NotRequired, Protocol, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils import token_counter
from litellm.litellm_core_utils.token_counter import (
    _count_content_list,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/token_counter.py#L869-L951
    _get_count_function,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/token_counter.py#L608-L612
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

    def __call__(self, _client: httpx.Client, url: str, **_options: JsonValue) -> httpx.Response:
        data = self.urls.get(url)
        if data is None:
            raise httpx.ConnectError("the helper has no bytes for this URL")
        _header, encoded = data.split(",", 1)
        return httpx.Response(200, content=base64.b64decode(encoded))


def run(input_value: Input) -> list[int]:
    settings = DEFAULT_SETTINGS.copy()
    given = input_value.get("settings")
    if given is not None:
        unknown = set(given) - set(DEFAULT_SETTINGS)
        if unknown:
            raise ValueError(f"unsupported image setting {min(unknown)}")
        settings.update(given)
    urls = input_value.get("urls")
    # the settings are plain module constants, and one helper process answers one case
    token_counter.DEFAULT_IMAGE_WIDTH = settings["default_width"]  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L133
    token_counter.DEFAULT_IMAGE_HEIGHT = settings["default_height"]  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L134
    token_counter.DEFAULT_IMAGE_TOKEN_COUNT = settings["default_token_count"]  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L99
    token_counter.MAX_SHORT_SIDE_FOR_IMAGE_HIGH_RES = settings["max_short_side"]  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L443
    token_counter.MAX_LONG_SIDE_FOR_IMAGE_HIGH_RES = settings["max_long_side"]  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L444
    token_counter.safe_get = Answers({} if urls is None else urls)  # type: ignore[attr-defined, assignment]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/url_utils.py#L421
    return [
        count_content(
            count_function=_get_count_function(DEFAULT_MODEL),
            content_list=call["content"],
            use_default_image_token_count=call.get("use_default_image_token_count") is True,
            default_token_count=call.get("default_token_count"),
        )
        for call in input_value["calls"]
    ]
