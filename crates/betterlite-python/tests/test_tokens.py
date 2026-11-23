import asyncio
from pathlib import Path
from typing import Final, Literal, TypedDict

import litellm
import pytest
from litellm.litellm_core_utils.token_counter import (
    calculate_img_tokens,
    calculate_tiles_needed,
    get_image_dimensions,
    get_image_type,
    high_detail_image_token_upper_bound,
    openai_tokenizer_encoding_name,
    resize_image_high_res,
    uses_legacy_message_accounting,
)
from litellm.litellm_core_utils.tokenizer import HuggingFace
from litellm.types.utils import Message as LiteLLMMessage
from pydantic import JsonValue

import betterlite
from betterlite import _native
from betterlite.types.utils import Message

TEXT: Final = "hello world"
TEXT_IDS: Final[list[int]] = [15339, 1917]
UNKNOWN_MODEL: Final = "betterlite-unknown-model"
LONG_TEXT: Final = "tokenizer " * 500_000
LONG_TEXT_MIN_TOKENS: Final = 1_000_000
MODELS: Final = ("gpt-4o", "gpt-3.5-turbo", "claude-3-5-sonnet-20241022", UNKNOWN_MODEL)
RUNTIME_MODELS: Final = ("gpt-3.5-turbo-0301", "gpt-3.5-turbo", "", UNKNOWN_MODEL)
PLAIN_MESSAGES: Final[list[dict[str, JsonValue]]] = [{"role": "user", "content": TEXT}]
ROLE_MESSAGES: Final[list[dict[str, JsonValue]]] = [
    {"role": "system", "content": "be brief"},
    {"role": "user", "content": TEXT},
    {"role": "assistant", "content": TEXT},
]
TOOLS: Final[list[dict[str, JsonValue]]] = [
    {
        "type": "function",
        "function": {"name": "get_weather", "parameters": {"type": "object"}},
    }
]
TOOL_CHOICES: Final[list[dict[str, JsonValue] | str]] = [
    {"type": "function", "function": {"name": "get_weather"}},
    "none",
]
MODIFIED_MESSAGES: Final[list[dict[str, JsonValue]]] = [{"role": "user", "content": TEXT}]
MODIFIED_CASES: Final[list[tuple[str, int, float | None, float | None]]] = [
    ("gpt-4o", 55, None, None),
    (UNKNOWN_MODEL, 55, None, None),
    ("ai21.j2-mid-v1", 20_000, None, None),
    ("ai21.j2-mid-v1", 20_000, None, 1000.0),
]
WORD_LEVEL: Final = (
    '{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],'
    '"normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":null,'
    '"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"hello":1,'
    '"world":2},"unk_token":"[UNK]"}}'
)
MISSING_REPOSITORY: Final = "betterlite-nonexistent/repository"
OFFLOAD_TIMEOUT: Final = 30.0
PNG_DATA: Final = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAABNIAAAI3"
GIF_DATA: Final = "data:image/gif;base64,R0lGODlhKAAeAA=="
PNG_SIZE: Final[tuple[int, int]] = (1234, 567)
GIF_SIZE: Final[tuple[int, int]] = (40, 30)
PNG_AUTO_TOKENS: Final = 85
PNG_HIGH_TOKENS: Final = 1445
GIF_HIGH_TOKENS: Final = 255
LARGE_BASE_TOKENS: Final = 100
LARGE_BASE_HIGH_TOKENS: Final = 1700
DEFAULT_IMAGE_TOKENS: Final = 250
DEFAULT_TILES: Final = 4
CUSTOM_TILES: Final = 96
HEADERS: Final[list[tuple[bytes, str | None]]] = [
    (b"\x89PNG\r\n\x1a\n\0\0\0\0\0\0\0\0\0\0", "png"),
    (b"GIF89a\x28\x00\x1e\x00", "gif"),
    (b"\xff\xd8\xff\x00\0\0", "jpeg"),
    (b"\x00\x00\x00\x18ftypheic", "heic"),
    (b"RIFF\x1e\x00\x00\x00WEBPVP8X", "webp"),
    (b"not an image", None),
    (b"", None),
]
IMAGE_MODES: Final[tuple[Literal["low", "high", "auto"], ...]] = ("low", "high", "auto")
RESIZE_CASES: Final[list[tuple[int, int]]] = [(3000, 1000), (768, 768), (1000, 3000), (2000, 500)]


class LiteLLMTokenizer(TypedDict):
    type: str
    tokenizer: HuggingFace


def test_counter_plain_text() -> None:
    for model in MODELS:
        ours = betterlite.token_counter(model=model, text=TEXT)
        theirs: int = litellm.token_counter(model=model, text=TEXT)
        assert ours == theirs, model
        assert ours == len(TEXT_IDS)


def test_counter_list_text() -> None:
    parts = ["hello ", "world"]
    ours = betterlite.token_counter(model="gpt-4o", text=parts)
    theirs: int = litellm.token_counter(model="gpt-4o", text=parts)
    assert ours == theirs
    assert ours == len(TEXT_IDS)


@pytest.mark.parametrize("message", ROLE_MESSAGES)
def test_counter_message_roles(message: dict[str, JsonValue]) -> None:
    ours = betterlite.token_counter(model="gpt-4o", messages=[message])
    theirs: int = litellm.token_counter(model="gpt-4o", messages=[message])
    assert ours == theirs


def test_counter_message_objects() -> None:
    ours = betterlite.token_counter(
        model="gpt-4o",
        messages=[Message(role="user", content=TEXT)],
    )
    theirs: int = litellm.token_counter(
        model="gpt-4o",
        messages=[LiteLLMMessage(role="user", content=TEXT)],
    )
    assert ours == theirs


def test_counter_tool_list() -> None:
    ours = betterlite.token_counter(model="gpt-4o", messages=PLAIN_MESSAGES, tools=TOOLS)
    theirs: int = litellm.token_counter(model="gpt-4o", messages=PLAIN_MESSAGES, tools=TOOLS)
    assert ours == theirs


def test_counter_tool_choice() -> None:
    for choice in TOOL_CHOICES:
        ours = betterlite.token_counter(
            model="gpt-4o",
            messages=PLAIN_MESSAGES,
            tool_choice=choice,
        )
        theirs: int = litellm.token_counter(
            model="gpt-4o",
            messages=PLAIN_MESSAGES,
            tool_choice=choice,
        )
        assert ours == theirs, choice


def test_counter_response_tokens() -> None:
    for flag in (False, True, None):
        ours = betterlite.token_counter(
            model="gpt-4o",
            messages=PLAIN_MESSAGES,
            count_response_tokens=flag,
            tools=TOOLS,
        )
        theirs: int = litellm.token_counter(
            model="gpt-4o",
            messages=PLAIN_MESSAGES,
            count_response_tokens=flag,
            tools=TOOLS,
        )
        assert ours == theirs, flag


def test_counter_long_text() -> None:
    ours = betterlite.token_counter(model="gpt-4o", text=LONG_TEXT)
    theirs: int = litellm.token_counter(model="gpt-4o", text=LONG_TEXT)
    assert ours == theirs
    assert ours > LONG_TEXT_MIN_TOKENS


def test_counter_custom_tokenizer() -> None:
    their_tokenizer: LiteLLMTokenizer = litellm.create_tokenizer(WORD_LEVEL)
    ours = betterlite.token_counter(
        text=TEXT,
        custom_tokenizer=betterlite.create_tokenizer(WORD_LEVEL),
    )
    their_count: int = litellm.token_counter(text=TEXT, custom_tokenizer=their_tokenizer)
    assert ours == their_count
    assert ours == len(TEXT.split())


def test_counter_unknown_type() -> None:
    loaded = betterlite.create_tokenizer(WORD_LEVEL)
    their_tokenizer: LiteLLMTokenizer = litellm.create_tokenizer(WORD_LEVEL)
    unsupported: betterlite.CustomTokenizer = {
        "type": "nonsense",
        "tokenizer": loaded["tokenizer"],
    }
    with pytest.raises(ValueError, match="Unsupported tokenizer type"):
        betterlite.token_counter(text=TEXT, custom_tokenizer=unsupported)
    with pytest.raises(ValueError, match="Unsupported tokenizer type"):
        litellm.token_counter(
            text=TEXT,
            custom_tokenizer={"type": "nonsense", "tokenizer": their_tokenizer["tokenizer"]},
        )


def test_counter_disabled() -> None:
    betterlite.disable_token_counter = True
    litellm.disable_token_counter = True
    try:
        ours = betterlite.token_counter(text=TEXT)
        theirs: int = litellm.token_counter(text=TEXT)
        assert ours == theirs
        assert ours == 0
    finally:
        betterlite.disable_token_counter = False
        litellm.disable_token_counter = False


def test_counter_value_errors() -> None:
    with pytest.raises(ValueError, match="text and messages cannot both be set"):
        betterlite.token_counter(text=TEXT, messages=PLAIN_MESSAGES)
    with pytest.raises(ValueError, match="text and messages cannot both be set"):
        litellm.token_counter(text=TEXT, messages=PLAIN_MESSAGES)
    with pytest.raises(ValueError, match="Either text or messages must be provided"):
        betterlite.token_counter()
    with pytest.raises(ValueError, match="Either text or messages must be provided"):
        litellm.token_counter()
    with pytest.raises(
        ValueError, match="tools or tool_choice cannot be set if using text"
    ):
        betterlite.token_counter(text=TEXT, tools=TOOLS)
    with pytest.raises(
        ValueError, match="tools or tool_choice cannot be set if using text"
    ):
        litellm.token_counter(text=TEXT, tools=TOOLS)


def test_counter_encoding_name() -> None:
    for model in (*MODELS, "gpt-4"):
        ours = betterlite.openai_tokenizer_encoding_name(model)
        assert ours == openai_tokenizer_encoding_name(model), model


def test_counter_legacy_accounting() -> None:
    for model in RUNTIME_MODELS:
        ours = betterlite.uses_legacy_message_accounting(model)
        assert ours == uses_legacy_message_accounting(model), model


def test_counter_offload() -> None:
    offloaded = betterlite.offload_token_count(betterlite.token_counter)
    counted = asyncio.run(asyncio.wait_for(offloaded(text=TEXT), timeout=OFFLOAD_TIMEOUT))
    assert counted == betterlite.token_counter(text=TEXT)


def test_codec_default_encoding() -> None:
    ours = betterlite.encode(text=TEXT)
    their_ids: list[int] = litellm.encode(text=TEXT)
    assert ours == their_ids
    assert ours == TEXT_IDS
    their_text: str = litellm.decode(tokens=TEXT_IDS)
    assert betterlite.decode(tokens=ours) == their_text == TEXT


def test_codec_ignores_model() -> None:
    ours = betterlite.encode(model="gpt-4o", text=TEXT)
    their_ids: list[int] = litellm.encode(model="gpt-4o", text=TEXT)
    assert ours == their_ids
    assert ours == TEXT_IDS


def test_codec_ignores_skip_flag() -> None:
    ours = betterlite.decode(tokens=TEXT_IDS, skip_special_tokens=False)
    their_text: str = litellm.decode(tokens=TEXT_IDS, skip_special_tokens=False)
    assert ours == their_text
    assert ours == betterlite.decode(tokens=TEXT_IDS)


def test_codec_custom_round_trip() -> None:
    tokenizer = betterlite.create_tokenizer(WORD_LEVEL)
    their_tokenizer: LiteLLMTokenizer = litellm.create_tokenizer(WORD_LEVEL)
    ours = betterlite.encode(text=TEXT, custom_tokenizer=tokenizer)
    their_ids: list[int] = litellm.encode(text=TEXT, custom_tokenizer=their_tokenizer)
    assert ours == their_ids
    assert betterlite.decode(tokens=ours, custom_tokenizer=tokenizer) == TEXT


def test_codec_special_flag_raises() -> None:
    tokenizer = betterlite.create_tokenizer(WORD_LEVEL)
    with pytest.raises(_native.NativeTokensError, match="skip_special_tokens"):
        betterlite.decode(tokens=[1, 2], custom_tokenizer=tokenizer, skip_special_tokens=False)


def test_codec_offline_hub(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setenv("HF_HUB_OFFLINE", "1")
    monkeypatch.setenv("HF_HOME", str(tmp_path))
    with pytest.raises(_native.NativeTokensError, match=MISSING_REPOSITORY):
        betterlite.create_pretrained_tokenizer(MISSING_REPOSITORY)


def test_image_type() -> None:
    for header, expected in HEADERS:
        assert betterlite.get_image_type(header) == get_image_type(header) == expected


def test_image_dimensions() -> None:
    theirs: tuple[int, int] = get_image_dimensions(PNG_DATA)
    assert betterlite.get_image_dimensions(PNG_DATA) == theirs == PNG_SIZE
    gif_theirs: tuple[int, int] = get_image_dimensions(GIF_DATA)
    assert betterlite.get_image_dimensions(GIF_DATA) == gif_theirs == GIF_SIZE


def test_image_resize() -> None:
    for width, height in RESIZE_CASES:
        theirs: tuple[int, int] = resize_image_high_res(width, height)
        assert betterlite.resize_image_high_res(width, height) == theirs, (width, height)


def test_image_tiles() -> None:
    theirs: int = calculate_tiles_needed(800, 600)
    assert betterlite.calculate_tiles_needed(800, 600) == theirs == DEFAULT_TILES
    custom: int = calculate_tiles_needed(800, 600, 100, 50)
    assert betterlite.calculate_tiles_needed(800, 600, 100, 50) == custom == CUSTOM_TILES


def test_image_bound() -> None:
    theirs: int = high_detail_image_token_upper_bound()
    ours = betterlite.high_detail_image_token_upper_bound()
    assert ours == theirs == PNG_HIGH_TOKENS
    custom: int = high_detail_image_token_upper_bound(LARGE_BASE_TOKENS)
    ours_custom = betterlite.high_detail_image_token_upper_bound(LARGE_BASE_TOKENS)
    assert ours_custom == custom == LARGE_BASE_HIGH_TOKENS


def test_image_tokens() -> None:
    for mode in IMAGE_MODES:
        theirs: int = calculate_img_tokens(PNG_DATA, mode)
        assert betterlite.calculate_img_tokens(PNG_DATA, mode) == theirs, mode
    assert betterlite.calculate_img_tokens(PNG_DATA) == PNG_AUTO_TOKENS
    assert betterlite.calculate_img_tokens(PNG_DATA, "high") == PNG_HIGH_TOKENS
    assert betterlite.calculate_img_tokens(GIF_DATA, "high") == GIF_HIGH_TOKENS
    assert betterlite.calculate_img_tokens(PNG_DATA, "low") == PNG_AUTO_TOKENS


def test_image_default_count() -> None:
    theirs: int = calculate_img_tokens(PNG_DATA, use_default_image_token_count=True)
    ours = betterlite.calculate_img_tokens(PNG_DATA, use_default_image_token_count=True)
    assert ours == theirs == DEFAULT_IMAGE_TOKENS


def test_modified_max_tokens() -> None:
    for model, user_max, buffer_perc, buffer_num in MODIFIED_CASES:
        theirs: int | None = litellm.get_modified_max_tokens(
            model,
            model,
            MODIFIED_MESSAGES,
            user_max,
            buffer_perc,
            buffer_num,
        )
        ours = betterlite.get_modified_max_tokens(
            model,
            model,
            MODIFIED_MESSAGES,
            user_max,
            buffer_perc,
            buffer_num,
        )
        assert ours == theirs, model
