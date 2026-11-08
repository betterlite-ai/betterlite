import os

os.environ["LITELLM_RUST"] = "false"

from typing import Protocol, TypeAlias, cast

import litellm
import tiktoken
from litellm import utils
from litellm.main import _get_encoding  # pyright: ignore[reportPrivateUsage]
from litellm.types.llms.openai import ChatCompletionNamedToolChoiceParam, ChatCompletionToolParam
from litellm.types.utils import SelectTokenizerResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class TokenCounter(Protocol):
    def __call__(  # noqa: PLR0913 - mirrors LiteLLM's token_counter keyword signature
        self,
        *,
        model: str = ...,
        custom_tokenizer: dict[str, object] | SelectTokenizerResponse | None = ...,
        text: str | list[str] | None = ...,
        messages: list[dict[str, Json]] | None = ...,
        count_response_tokens: bool | None = ...,
        tools: list[ChatCompletionToolParam] | None = ...,
        tool_choice: ChatCompletionNamedToolChoiceParam | None = ...,
    ) -> int: ...


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _text(input_value: dict[str, Json]) -> str:
    value = input_value.get("text")
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return "".join(item for item in value if isinstance(item, str))
    raise ValueError("text must be a string or a list of strings")


def _optional_text(input_value: dict[str, Json]) -> str | None:
    if input_value.get("text") is None:
        return None
    return _text(input_value)


def _messages(input_value: dict[str, Json]) -> list[dict[str, Json]] | None:
    value = input_value.get("messages")
    if value is None:
        return None
    if not isinstance(value, list):
        raise ValueError("messages must be a list")
    messages: list[dict[str, Json]] = []
    for message in value:
        if not isinstance(message, dict):
            raise ValueError("each message must be an object")
        messages.append(message)
    return messages


def _tools(input_value: dict[str, Json]) -> list[ChatCompletionToolParam] | None:
    value = input_value.get("tools")
    if value is None:
        return None
    if not isinstance(value, list):
        raise ValueError("tools must be a list")
    return cast("list[ChatCompletionToolParam]", value)


def _tool_choice(input_value: dict[str, Json]) -> ChatCompletionNamedToolChoiceParam | None:
    value = input_value.get("tool_choice")
    if value is None:
        return None
    return cast("ChatCompletionNamedToolChoiceParam", value)


def _custom_tokenizer(input_value: dict[str, Json]) -> object:
    value = input_value.get("custom_tokenizer")
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ValueError("custom_tokenizer must be an object")
    kind = value.get("type")
    if kind == "huggingface_tokenizer":
        document = _string(value.get("tokenizer_json"), "tokenizer_json")
        return utils.create_tokenizer(document)
    if kind == "openai_tokenizer":
        return {"type": "openai_tokenizer", "tokenizer": tiktoken.get_encoding("cl100k_base")}
    raise ValueError(f"unknown custom tokenizer type: {kind!r}")


def _default_encoding_count(input_value: dict[str, Json]) -> int:
    encoding = cast(tiktoken.Encoding, _get_encoding())
    return len(encoding.encode(_text(input_value), disallowed_special=()))


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "default_encoding":
        return _default_encoding_count(input_value)
    if function != "token_counter":
        raise ValueError(f"unknown token count function: {function!r}")
    litellm.disable_token_counter = input_value.get("disable_token_counter") is True
    counter = cast(TokenCounter, litellm.token_counter)
    custom_tokenizer = cast(
        "dict[str, object] | SelectTokenizerResponse | None", _custom_tokenizer(input_value)
    )
    return counter(
        model=_string(input_value.get("model", ""), "model"),
        custom_tokenizer=custom_tokenizer,
        text=_optional_text(input_value),
        messages=_messages(input_value),
        count_response_tokens=input_value.get("count_response_tokens") is True,
        tools=_tools(input_value),
        tool_choice=_tool_choice(input_value),
    )
