from collections.abc import Callable
from types import SimpleNamespace
from typing import TypeAlias, cast

from litellm.integrations.websearch_interception.transformation import WebSearchTransformation
from litellm.llms.base_llm.search.transformation import SearchResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation", "detect")
    if operation == "native":
        search_value = input_value.get("search_response")
        search = SearchResponse.model_validate(search_value) if search_value is not None else None
        return cast(
            Json, WebSearchTransformation.build_web_search_tool_result_block("call", search)
        )
    if operation == "format":
        return WebSearchTransformation.format_search_response(
            SearchResponse.model_validate(input_value["search_response"])
        )
    response_format = cast(str, input_value.get("format", "anthropic"))
    response = input_value.get("response", {})
    model_response = (
        object_response(response) if input_value.get("object_response", False) else response
    )
    detect = cast(
        Callable[[object, bool, str], tuple[bool, list[dict[str, Json]]]],
        WebSearchTransformation.transform_request,
    )
    found, calls = detect(model_response, bool(input_value.get("stream", False)), response_format)
    output: dict[str, Json] = {"has_websearch": found, "tool_calls": cast(Json, calls)}
    if input_value.get("transform", False):
        transform = cast(
            Callable[
                [list[dict[str, Json]], list[str], str, list[dict[str, Json]]],
                tuple[dict[str, Json], Json],
            ],
            WebSearchTransformation.transform_response,
        )
        assistant, messages = transform(
            calls,
            cast(list[str], input_value.get("results", [])),
            response_format,
            cast(list[dict[str, Json]], input_value.get("thinking", [])),
        )
        output["assistant"] = cast(Json, assistant)
        output["messages"] = messages
    return output


def object_response(value: Json) -> object:
    if isinstance(value, list):
        return [object_response(item) for item in value]
    if isinstance(value, dict):
        fields = {
            key: object_response(item)
            if key in ("content", "choices", "output", "message", "function", "tool_calls")
            else item
            for key, item in value.items()
        }
        return SimpleNamespace(**fields)
    return value
