from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.litellm_core_utils.prompt_templates import factory
from litellm.litellm_core_utils.prompt_templates.factory import (
    _ANTHROPIC_TOOL_USE_ID_INVALID_CHARS,  # pyright: ignore[reportPrivateUsage]
    _BEDROCK_TOOL_USE_ID_INVALID_CHARS,  # pyright: ignore[reportPrivateUsage]
    _find_server_tool_result,  # pyright: ignore[reportPrivateUsage]
    _is_anthropic_document_data_uri,  # pyright: ignore[reportPrivateUsage]
    _replace_invalid_tool_use_id_chars,  # pyright: ignore[reportPrivateUsage]
    _sanitize_anthropic_tool_use_id,  # pyright: ignore[reportPrivateUsage]
    convert_function_to_anthropic_tool_invoke,  # pyright: ignore[reportUnknownVariableType]
    convert_generic_image_chunk_to_openai_image_obj,
    convert_to_anthropic_image_obj,
    convert_to_anthropic_tool_result,
    create_anthropic_image_param,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.types.llms.openai import (
    AllMessageValues,
    ChatCompletionFunctionMessage,
    ChatCompletionToolMessage,
)
from litellm.types.utils import GenericImageParsingChunk

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _MappedConvert:
    def __init__(self, mapping: dict[str, str]) -> None:
        self.mapping = mapping

    def __call__(self, url: str) -> str:
        return self.mapping[url]


class _FixedUuid:
    def __init__(self, value: str) -> None:
        self.value = value

    def uuid4(self) -> str:
        return self.value


def _values(input_value: dict[str, Json]) -> list[str]:
    return cast("list[str]", input_value["values"])


def _sanitize_ids(input_value: dict[str, Json]) -> Json:
    return [_sanitize_anthropic_tool_use_id(value) for value in _values(input_value)]


def _document_uris(input_value: dict[str, Json]) -> Json:
    return [_is_anthropic_document_data_uri(value) for value in _values(input_value)]


def _replace_ids(input_value: dict[str, Json]) -> Json:
    pattern = (
        _ANTHROPIC_TOOL_USE_ID_INVALID_CHARS
        if input_value["alphabet"] == "anthropic"
        else _BEDROCK_TOOL_USE_ID_INVALID_CHARS
    )
    return [_replace_invalid_tool_use_id_chars(value, pattern) for value in _values(input_value)]


def _tool_result(input_value: dict[str, Json]) -> Json:
    message = cast(
        "ChatCompletionToolMessage | ChatCompletionFunctionMessage",
        cast(AllMessageValues, input_value["message"]),
    )
    mapping = cast("dict[str, str]", input_value.get("data_urls", {}))
    fixed_id = cast(str, input_value["id"])
    force_base64 = cast(bool, input_value.get("force_base64", False))
    with (
        patch.object(factory, "convert_url_to_base64", _MappedConvert(mapping)),
        patch.object(factory, "uuid", _FixedUuid(fixed_id)),
    ):
        return cast(Json, convert_to_anthropic_tool_result(message, force_base64=force_base64))


def _function_invoke(input_value: dict[str, Json]) -> Json:
    function_call = cast("dict[str, object]", input_value["function_call"])
    fixed_id = cast(str, input_value["id"])
    with patch.object(factory, "uuid", _FixedUuid(fixed_id)):
        return cast(Json, convert_function_to_anthropic_tool_invoke(function_call))


def _server_tool_result(input_value: dict[str, Json]) -> Json:
    return cast(
        Json,
        _find_server_tool_result(
            cast(str, input_value["tool_id"]),
            cast("list[object] | None", input_value.get("web_search_results")),
            cast("list[object] | None", input_value.get("tool_results")),
        ),
    )


def _image_chunk_object(input_value: dict[str, Json]) -> Json:
    chunk = cast(GenericImageParsingChunk, input_value["chunk"])
    return convert_generic_image_chunk_to_openai_image_obj(chunk)


def _anthropic_image_chunk(input_value: dict[str, Json]) -> Json:
    mapping = cast("dict[str, str]", input_value.get("data_urls", {}))
    with patch.object(factory, "convert_url_to_base64", _MappedConvert(mapping)):
        return cast(
            Json,
            convert_to_anthropic_image_obj(
                cast(str, input_value["openai_image_url"]),
                cast("str | None", input_value.get("format")),
            ),
        )


def _image_param(input_value: dict[str, Json]) -> Json:
    mapping = cast("dict[str, str]", input_value.get("data_urls", {}))
    with patch.object(factory, "convert_url_to_base64", _MappedConvert(mapping)):
        return cast(
            Json,
            create_anthropic_image_param(
                cast("str | dict[str, Json]", input_value["image_url_input"]),
                format=cast("str | None", input_value.get("format")),
                is_bedrock_invoke=cast(bool, input_value.get("is_bedrock_invoke", False)),
            ),
        )


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "sanitize_anthropic_ids":
        result = _sanitize_ids(input_value)
    elif function == "replace_invalid_id_chars":
        result = _replace_ids(input_value)
    elif function == "document_data_uris":
        result = _document_uris(input_value)
    elif function == "tool_result":
        result = _tool_result(input_value)
    elif function == "function_invoke":
        result = _function_invoke(input_value)
    elif function == "server_tool_result":
        result = _server_tool_result(input_value)
    elif function == "image_chunk_object":
        result = _image_chunk_object(input_value)
    elif function == "anthropic_image_chunk":
        result = _anthropic_image_chunk(input_value)
    elif function == "image_param":
        result = _image_param(input_value)
    else:
        raise ValueError(f"unknown anthropic_results function: {function!r}")
    return result
