import contextlib
from collections.abc import Sequence
from json import dumps
from typing import Final

from pydantic import JsonValue
from typing_extensions import TypedDict

from betterlite import _native

HUGGING_FACE: Final = "huggingface_tokenizer"


class NativeTokenizer:
    # a tokenizer loaded in rust; the handle stays valid until this object is collected
    __slots__ = ("tokenizer_id",)

    def __init__(self, tokenizer_id: int) -> None:
        self.tokenizer_id = tokenizer_id

    def __del__(self) -> None:
        # the interpreter may already have torn the native module down when this runs at shutdown
        with contextlib.suppress(AttributeError, NameError):
            _native.free_tokenizer(self.tokenizer_id)


class CustomTokenizer(TypedDict):
    type: str
    tokenizer: NativeTokenizer


# LiteLLM's _get_count_function raises this text for a tokenizer dict it does not know
def tokenizer_handle(custom_tokenizer: CustomTokenizer) -> int:
    if custom_tokenizer["type"] != HUGGING_FACE:
        raise ValueError("Unsupported tokenizer type")
    return custom_tokenizer["tokenizer"].tokenizer_id


def request_with_tokenizer(
    model: str,
    custom_tokenizer: CustomTokenizer | None,
) -> dict[str, JsonValue]:
    request: dict[str, JsonValue] = {"model": model}
    if custom_tokenizer is not None:
        request["custom_tokenizer"] = tokenizer_handle(custom_tokenizer)
    return request


def encode(
    model: str = "",
    text: str = "",
    custom_tokenizer: CustomTokenizer | None = None,
) -> list[int]:
    request: dict[str, JsonValue] = request_with_tokenizer(model, custom_tokenizer)
    request["text"] = text
    return _native.encode_text(dumps(request))


def decode(
    model: str = "",
    tokens: Sequence[int] = (),
    custom_tokenizer: CustomTokenizer | None = None,
    skip_special_tokens: bool = True,
) -> str:
    request: dict[str, JsonValue] = request_with_tokenizer(model, custom_tokenizer)
    tokens_json: list[JsonValue] = []
    tokens_json.extend(tokens)
    request["tokens"] = tokens_json
    request["skip_special_tokens"] = skip_special_tokens
    return _native.decode_tokens(dumps(request))


def create_pretrained_tokenizer(
    identifier: str,
    revision: str = "main",
    auth_token: str | None = None,
) -> CustomTokenizer:
    request: dict[str, JsonValue] = {
        "kind": "pretrained",
        "identifier": identifier,
        "revision": revision,
    }
    if auth_token is not None:
        request["auth_token"] = auth_token
    handle = _native.load_tokenizer(dumps(request))
    return {"type": HUGGING_FACE, "tokenizer": NativeTokenizer(handle)}


def create_tokenizer(json: str) -> CustomTokenizer:
    request: dict[str, JsonValue] = {"kind": "json", "json": json}
    handle = _native.load_tokenizer(dumps(request))
    return {"type": HUGGING_FACE, "tokenizer": NativeTokenizer(handle)}
