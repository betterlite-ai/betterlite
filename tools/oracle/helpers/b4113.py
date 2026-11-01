from collections.abc import Callable, Mapping, Sequence
from types import SimpleNamespace
from typing import TypeAlias, cast

from litellm.exceptions import BadRequestError, RateLimitError
from litellm.integrations.websearch_interception.handler import WebSearchInterceptionLogger
from litellm.integrations.websearch_interception.transformation import WebSearchTransformation
from litellm.llms.base_llm.search.transformation import SearchResponse
from litellm.types.integrations.websearch_interception import (
    SearchFailed,
    SearchSucceeded,
    WebSearchToolResultErrorCode,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_Detect: TypeAlias = Callable[[object, bool, str], tuple[bool, list[dict[str, Json]]]]
_TransformResponse: TypeAlias = Callable[
    [list[dict[str, Json]], Sequence[str], str, list[dict[str, Json]]],
    tuple[dict[str, Json], Json],
]
_BuildNativeResultBlocks: TypeAlias = Callable[
    [list[dict[str, Json]], Sequence[SearchSucceeded | SearchFailed]],
    tuple[Mapping[str, object], ...],
]
_InjectNativeBlocks: TypeAlias = Callable[[object, Sequence[Mapping[str, object]]], object]


def _exception_from_error(error: dict[str, Json]) -> BaseException:
    family = error.get("family")
    if family == "rate_limit":
        context = cast(dict[str, Json], error["context"])
        return RateLimitError(
            cast(str, context["message"]),
            cast("str | None", context.get("provider")),
            cast("str | None", context.get("model")),
            category=cast(str, error["category"]),
        )
    if family == "bad_request":
        return BadRequestError(
            cast(str, error["message"]),
            cast("str | None", error.get("model")),
            cast("str | None", error.get("provider")),
        )
    return Exception(cast(str, error.get("message", "")))


def _search_response(value: Json) -> SearchResponse | None:
    return SearchResponse.model_validate(value) if value is not None else None


def _outcome_from_json(item: dict[str, Json]) -> SearchSucceeded | SearchFailed:
    if "text" in item:
        response = _search_response(item.get("response"))
        return SearchSucceeded(text=cast(str, item["text"]), response=response)
    error_code = cast(WebSearchToolResultErrorCode, item["error_code"])
    return SearchFailed(error_code=error_code, message=cast(str, item["message"]))


def _outcome_to_json(outcome: SearchSucceeded | SearchFailed) -> Json:
    if isinstance(outcome, SearchSucceeded):
        response = outcome.response.model_dump() if outcome.response is not None else None
        return {"text": outcome.text, "response": response}
    return {"error_code": outcome.error_code, "message": outcome.message}


def _detect(response_value: Json, response_format: str) -> list[dict[str, Json]]:
    detect = cast(_Detect, WebSearchTransformation.transform_request)
    stream = False
    _found, calls = detect(response_value, stream, response_format)
    return calls


def _search_outcome(input_value: dict[str, Json]) -> Json:
    if "error" in input_value:
        result: object = _exception_from_error(cast(dict[str, Json], input_value["error"]))
    else:
        text = cast(str, input_value.get("text", ""))
        result = (text, _search_response(input_value.get("search_response")))
    outcome = WebSearchTransformation.search_outcome(result)
    return _outcome_to_json(outcome)


def _inject_native_blocks(input_value: dict[str, Json]) -> Json:
    native_blocks = cast(list[Mapping[str, object]], input_value["native_blocks"])
    response_value = input_value["response"]
    inject = cast(
        _InjectNativeBlocks,
        WebSearchInterceptionLogger._inject_native_blocks,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    if input_value.get("as_object", False):
        response_obj = SimpleNamespace(**cast(dict[str, Json], response_value))
        return cast(Json, vars(inject(response_obj, native_blocks)))
    return cast(Json, inject(response_value, native_blocks))


def run(input_value: dict[str, Json]) -> Json:  # noqa: PLR0911 - one branch per b4113 operation
    operation = input_value["operation"]
    if operation == "search_error_code":
        error = _exception_from_error(cast(dict[str, Json], input_value["error"]))
        return WebSearchTransformation.search_error_code(error)
    if operation == "search_outcome":
        return _search_outcome(input_value)
    if operation == "format_search_response":
        return WebSearchTransformation.format_search_response(
            SearchResponse.model_validate(input_value["search_response"])
        )
    if operation == "transform_response":
        response_format = cast(str, input_value.get("format", "anthropic"))
        calls = _detect(input_value["response"], response_format)
        transform = cast(_TransformResponse, WebSearchTransformation.transform_response)
        assistant, messages = transform(
            calls,
            cast(list[str], input_value["search_results"]),
            response_format,
            cast(list[dict[str, Json]], input_value.get("thinking", [])),
        )
        return [cast(Json, assistant), messages]
    if operation == "build_web_search_tool_result_block":
        tool_use_id = cast(str, input_value["tool_use_id"])
        search_response = _search_response(input_value.get("search_response"))
        block = WebSearchTransformation.build_web_search_tool_result_block(
            tool_use_id, search_response
        )
        return cast(Json, block)
    if operation == "build_web_search_tool_result_error_block":
        tool_use_id = cast(str, input_value["tool_use_id"])
        error_code = cast(WebSearchToolResultErrorCode, input_value["error_code"])
        error_block = WebSearchTransformation.build_web_search_tool_result_error_block(
            tool_use_id, error_code
        )
        return cast(Json, error_block)
    if operation == "native_result_pairs":
        response_format = cast(str, input_value.get("format", "anthropic"))
        calls = _detect(input_value["response"], response_format)
        raw_outcomes = cast(list[dict[str, Json]], input_value["outcomes"])
        outcomes = [_outcome_from_json(item) for item in raw_outcomes]
        build_pairs = cast(
            _BuildNativeResultBlocks,
            WebSearchInterceptionLogger._build_native_result_blocks,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        )
        blocks = build_pairs(calls, outcomes)
        return cast(Json, list(blocks))
    if operation == "inject_native_blocks":
        return _inject_native_blocks(input_value)
    raise ValueError(f"unknown b4113 operation: {operation}")
