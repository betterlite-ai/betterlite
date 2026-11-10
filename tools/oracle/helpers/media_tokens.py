import contextlib
import io
import os
from collections.abc import Callable
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

# LiteLLM reads LITELLM_RUST per call; false pins the Python tokenizer the oracle compares against.
os.environ["LITELLM_RUST"] = "false"

from litellm.litellm_core_utils import token_counter as token_counter_module

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class CountContentList(Protocol):
    def __call__(
        self,
        *,
        count_function: Callable[[str], int],
        content_list: list[Json],
        use_default_image_token_count: bool,
        default_token_count: int | None,
    ) -> int: ...


class GetCountFunction(Protocol):
    def __call__(
        self, model: str, custom_tokenizer: None = None
    ) -> Callable[[str], int]: ...


class GetModifiedMaxTokens(Protocol):
    def __call__(  # noqa: PLR0913 - mirrors LiteLLM's keyword signature
        self,
        *,
        model: str,
        base_model: str,
        messages: list[Json] | None,
        user_max_tokens: int | None,
        buffer_perc: float | None,
        buffer_num: float | None,
    ) -> int | None: ...


count_content_list = cast(
    CountContentList, token_counter_module.__dict__["_count_content_list"]
)
get_count_function = cast(
    GetCountFunction, token_counter_module.__dict__["_get_count_function"]
)
get_modified_max_tokens = cast(
    GetModifiedMaxTokens, token_counter_module.__dict__["get_modified_max_tokens"]
)
calculate_tiles_needed = cast(
    "Callable[..., int]", token_counter_module.calculate_tiles_needed
)

DEFAULT_MODEL = "gpt-3.5-turbo"
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


def _optional_integer(value: Json, name: str) -> int | None:
    if value is None:
        return None
    return _integer(value, name)


def _optional_float(value: Json, name: str) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{name} must be a number")
    return float(value)


def _boolean(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


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


def _content(call: dict[str, Json]) -> list[Json]:
    value = call.get("content")
    if not isinstance(value, list):
        raise ValueError("content must be a list")
    return value


def _messages(call: dict[str, Json]) -> list[Json] | None:
    value = call.get("messages")
    if value is None:
        return None
    if not isinstance(value, list):
        raise ValueError("messages must be a list")
    messages: list[Json] = []
    for message in value:
        if not isinstance(message, dict):
            raise ValueError("each message must be an object")
        messages.append(message)
    return messages


def _count_content(call: dict[str, Json], model: str) -> Json:
    use_default = call.get("use_default_image_token_count")
    uses_default = (
        False
        if use_default is None
        else _boolean(use_default, "use_default_image_token_count")
    )
    with contextlib.redirect_stdout(io.StringIO()):
        return count_content_list(
            count_function=get_count_function(model),
            content_list=_content(call),
            use_default_image_token_count=uses_default,
            default_token_count=_optional_integer(
                call.get("default_token_count"), "default_token_count"
            ),
        )


def _modified_max_tokens(call: dict[str, Json]) -> Json:
    with contextlib.redirect_stdout(io.StringIO()):
        return get_modified_max_tokens(
            model=_string(call.get("model"), "model"),
            base_model=_string(call.get("base_model"), "base_model"),
            messages=_messages(call),
            user_max_tokens=_optional_integer(call.get("user_max_tokens"), "user_max_tokens"),
            buffer_perc=_optional_float(call.get("buffer_perc"), "buffer_perc"),
            buffer_num=_optional_float(call.get("buffer_num"), "buffer_num"),
        )


def _call(call: dict[str, Json], model: str) -> Json:
    function = call.get("function")
    match function:
        case "count_content":
            return _count_content(call, model)
        case "modified_max_tokens":
            return _modified_max_tokens(call)
        case other:
            raise ValueError(f"unknown media_tokens function: {other!r}")


def run(input_value: dict[str, Json]) -> Json:
    settings = _settings(input_value)
    model = input_value.get("model")
    calls = _calls(input_value)
    with (
        patch.object(token_counter_module, "DEFAULT_IMAGE_WIDTH", settings["default_width"]),
        patch.object(token_counter_module, "DEFAULT_IMAGE_HEIGHT", settings["default_height"]),
        patch.object(
            token_counter_module, "DEFAULT_IMAGE_TOKEN_COUNT", settings["default_token_count"]
        ),
        patch.object(
            token_counter_module, "MAX_SHORT_SIDE_FOR_IMAGE_HIGH_RES", settings["max_short_side"]
        ),
        patch.object(
            token_counter_module, "MAX_LONG_SIDE_FOR_IMAGE_HIGH_RES", settings["max_long_side"]
        ),
        patch.object(
            calculate_tiles_needed,
            "__defaults__",
            (settings["tile_width"], settings["tile_height"]),
        ),
    ):
        return [
            _call(call, DEFAULT_MODEL if model is None else _string(model, "model"))
            for call in calls
        ]
