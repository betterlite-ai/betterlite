from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.litellm_core_utils.prompt_templates import factory
from litellm.litellm_core_utils.prompt_templates.factory import (
    _anthropic_content_element_factory,  # pyright: ignore[reportPrivateUsage]
    add_cache_control_to_content,  # pyright: ignore[reportUnknownVariableType]
    anthropic_infer_file_id_content_type,
    anthropic_process_openai_file_message,
    convert_to_anthropic_tool_invoke,
    select_anthropic_content_block_type_for_file,
)
from litellm.types.llms.openai import (
    AllMessageValues,
    ChatCompletionAssistantToolCall,
    ChatCompletionFileObject,
)
from litellm.types.utils import GenericImageParsingChunk

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _MappedConvert:
    def __init__(self, mapping: dict[str, str]) -> None:
        self.mapping = mapping

    def __call__(self, url: str) -> str:
        return self.mapping[url]


def _tool_invoke(input_value: dict[str, Json]) -> Json:
    tool_calls = cast(list[ChatCompletionAssistantToolCall], input_value["tool_calls"])
    web_search_results = cast("list[object]", input_value.get("web_search_results", []))
    tool_results = cast("list[object]", input_value.get("tool_results", []))
    return cast(
        Json,
        convert_to_anthropic_tool_invoke(
            tool_calls,
            web_search_results=web_search_results,
            tool_results=tool_results,
        ),
    )


def _cache_control(input_value: dict[str, Json]) -> Json:
    element = cast("dict[str, Json]", input_value["element"])
    original = cast(AllMessageValues, input_value["original"])
    return cast(Json, add_cache_control_to_content(element, original))


def _document_type(input_value: dict[str, Json]) -> Json:
    formats = cast("list[str]", input_value["formats"])
    file_ids = cast("list[str]", input_value["file_ids"])
    chunk = cast(GenericImageParsingChunk, input_value["chunk"])
    return {
        "select": [select_anthropic_content_block_type_for_file(value) for value in formats],
        "infer": [anthropic_infer_file_id_content_type(value) for value in file_ids],
        "factory": cast(Json, _anthropic_content_element_factory(chunk)),
    }


def _process_file_message(input_value: dict[str, Json]) -> Json:
    messages = cast(list[ChatCompletionFileObject], input_value["messages"])
    mapping = cast("dict[str, str]", input_value.get("data_urls", {}))
    with patch.object(factory, "convert_url_to_base64", _MappedConvert(mapping)):
        return [
            cast(Json, anthropic_process_openai_file_message(message))
            for message in messages
        ]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "convert_to_anthropic_tool_invoke":
        result = _tool_invoke(input_value)
    elif function == "add_cache_control_to_content":
        result = _cache_control(input_value)
    elif function == "document_type":
        result = _document_type(input_value)
    elif function == "process_openai_file_message":
        result = _process_file_message(input_value)
    else:
        raise ValueError(f"unknown anthropic_invokes function: {function!r}")
    return result
