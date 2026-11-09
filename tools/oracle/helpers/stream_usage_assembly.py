import os

os.environ["LITELLM_RUST"] = "false"

from collections.abc import Callable
from typing import Any, TypeAlias, cast
from unittest import mock

from litellm.litellm_core_utils.streaming_chunk_builder_utils import (
    ChunkProcessor,
    apply_grounding_request_counts,
    attach_cache_creation_token_details,
    capture_cache_creation_token_details,
)
from litellm.types.utils import (
    CacheCreationTokenDetails,
    Choices,
    ModelResponse,
    ModelResponseStream,
    PromptTokensDetailsWrapper,
    StreamingChoices,
    Usage,
)
from pydantic import ValidationError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class CountFailureError(Exception):
    pass


def prune(value: Json) -> Json:
    if isinstance(value, dict):
        return {key: prune(item) for key, item in value.items() if item is not None}
    if isinstance(value, list):
        return [prune(item) for item in value]
    return value


def detail_view(details: object) -> Json:
    if details is None:
        return None
    dumped = cast(Json, cast(Any, details).model_dump())  # pyright: ignore[reportAny]
    return prune(dumped)


def optional_attribute(value: object, name: str) -> Json:
    found = getattr(value, name, None)  # pyright: ignore[reportAny]
    return cast(Json, found)  # pyright: ignore[reportAny]


def usage_view(usage: Usage) -> Json:
    fields: dict[str, Json] = {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "cache_creation_input_tokens": optional_attribute(usage, "cache_creation_input_tokens"),
        "cache_read_input_tokens": optional_attribute(usage, "cache_read_input_tokens"),
        "completion_tokens_details": detail_view(usage.completion_tokens_details),
        "prompt_tokens_details": detail_view(usage.prompt_tokens_details),
        "server_tool_use": detail_view(optional_attribute(usage, "server_tool_use")),
        "cost": optional_attribute(usage, "cost"),
        "inference_geo": optional_attribute(usage, "inference_geo"),
        "speed": optional_attribute(usage, "speed"),
    }
    return prune(fields)


def details_input(value: Json) -> PromptTokensDetailsWrapper | None:
    return None if value is None else PromptTokensDetailsWrapper.model_validate(value)


def cache_details_input(value: Json) -> CacheCreationTokenDetails | None:
    return None if value is None else CacheCreationTokenDetails.model_validate(value)


def build_chunk(spec: dict[str, Json]) -> object:
    form = cast(str, spec.get("form", "dict"))
    choices = cast(list[Json], spec.get("choices", []))
    hidden = cast(dict[str, object], spec.get("_hidden_params", {}))
    if form == "dict":
        chunk: dict[str, Json] = {"choices": choices}
        if "_hidden_params" in spec:
            chunk["_hidden_params"] = cast(Json, hidden)
        if "usage" in spec:
            chunk["usage"] = spec["usage"]
        return chunk
    if form == "model_response_stream":
        response: ModelResponse | ModelResponseStream = ModelResponseStream(
            choices=cast(list[StreamingChoices], choices),
            id="chatcmpl-fixture",
            created=0,
            model="gpt-4o",
        )
    else:
        response = ModelResponse(
            choices=cast(list[Choices], choices),
            id="chatcmpl-fixture",
            created=0,
            model="gpt-4o",
        )
    if "usage" in spec:
        raw_usage = spec["usage"]
        response.usage = None if raw_usage is None else Usage.model_validate(raw_usage)  # pyright: ignore[reportAttributeAccessIssue]
    response._hidden_params = hidden  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
    return response


def make_counter(fixture: dict[str, Json]) -> tuple[Callable[..., int], list[Json]]:
    calls: list[Json] = []

    def fake(**kwargs: Json) -> int:
        text = kwargs.get("text")
        if text is not None:
            call: dict[str, Json] = {
                "kind": "text",
                "text": cast(str, text),
                "count_response_tokens": kwargs.get("count_response_tokens") is True,
            }
            if "model" in kwargs:
                call["model"] = cast(str, kwargs["model"])
            calls.append(call)
            if fixture.get("fail_text") is True:
                raise CountFailureError
            return cast(int, fixture.get("text", 0))
        raw_messages = kwargs.get("messages")
        messages_call: dict[str, Json] = {
            "kind": "messages",
            "model": cast(str, kwargs.get("model", "")),
            "messages": raw_messages,
        }
        calls.append(messages_call)
        if raw_messages is None or fixture.get("fail_messages") is True:
            raise CountFailureError
        return cast(int, fixture.get("messages", 0))

    return fake, calls


def prompt_callback(value: Json) -> Callable[[], int]:
    if value == "raise":

        def failing() -> int:
            raise CountFailureError

        return failing
    counted = cast(int, value)
    return lambda: counted


def calculate_case(arguments: dict[str, Json]) -> Json:
    raw_chunks = cast(list[dict[str, Json]], arguments.get("chunks", []))
    chunks = [build_chunk(spec) for spec in raw_chunks]
    guarded = ChunkProcessor(chunks) if chunks else ChunkProcessor.__new__(ChunkProcessor)
    processor = cast(Any, guarded)  # pyright: ignore[reportAny]
    fixture = cast(dict[str, Json], arguments.get("counter", {}))
    counter, calls = make_counter(fixture)
    verbose: list[Json] = []
    callback = (
        prompt_callback(arguments["count_prompt_tokens"])
        if "count_prompt_tokens" in arguments
        else None
    )
    try:
        with (
            mock.patch(
                "litellm.litellm_core_utils.streaming_chunk_builder_utils.token_counter", counter
            ),
            mock.patch(
                "litellm.litellm_core_utils.streaming_chunk_builder_utils.print_verbose",
                verbose.append,
            ),
        ):
            usage = cast(
                Usage,
                processor.calculate_usage(  # pyright: ignore[reportAny]
                    chunks=chunks,
                    model=cast(str, arguments["model"]),
                    completion_output=cast(str, arguments["completion_output"]),
                    messages=cast(list[dict[str, Json]] | None, arguments.get("messages")),
                    reasoning_tokens=cast(int | None, arguments.get("reasoning_tokens")),
                    count_prompt_tokens=callback,
                ),
            )
    except CountFailureError:
        return {"ok": False, "error_type": "count_failed"}
    except ValidationError:
        return {"ok": False, "error_type": "invalid_server_tool_use"}
    return {"ok": True, "usage": usage_view(usage), "counter_calls": calls, "verbose": verbose}


def reasoning_case(arguments: dict[str, Json]) -> Json:
    choices = cast(list[Choices], arguments.get("choices", []))
    fixture = cast(dict[str, Json], arguments.get("counter", {}))
    counter, calls = make_counter(fixture)
    processor = cast(Any, ChunkProcessor.__new__(ChunkProcessor))  # pyright: ignore[reportAny]
    try:
        with mock.patch(
            "litellm.litellm_core_utils.streaming_chunk_builder_utils.token_counter", counter
        ):
            counted = cast(
                int | None,
                processor.count_reasoning_tokens(  # pyright: ignore[reportAny]
                    ModelResponse(choices=choices)
                ),
            )
    except CountFailureError:
        return {"ok": False, "error_type": "count_failed"}
    return {"ok": True, "reasoning_tokens": counted, "counter_calls": calls}


def capture_case(arguments: dict[str, Json]) -> Json:
    prompt_details = details_input(arguments.get("prompt_tokens_details"))
    current = cache_details_input(arguments.get("current"))
    captured = capture_cache_creation_token_details(prompt_details, current)
    return {"ok": True, "details": detail_view(captured)}


def update_view(result: PromptTokensDetailsWrapper | None, original: object) -> Json:
    return {"ok": True, "same_object": result is original, "details": detail_view(result)}


def attach_case(arguments: dict[str, Json]) -> Json:
    prompt_details = details_input(arguments.get("prompt_tokens_details"))
    cache = cache_details_input(arguments.get("cache_creation_token_details"))
    return update_view(attach_cache_creation_token_details(prompt_details, cache), prompt_details)


def grounding_case(arguments: dict[str, Json]) -> Json:
    prompt_details = details_input(arguments.get("prompt_tokens_details"))
    result = apply_grounding_request_counts(
        prompt_details,
        cast(int | None, arguments.get("web_search_requests")),
        cast(int | None, arguments.get("google_maps_grounding_requests")),
    )
    return update_view(result, prompt_details)


def run(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    if function == "batch":
        return [run(case) for case in cast(list[dict[str, Json]], input_value["cases"])]
    if function == "calculate_usage":
        return calculate_case(input_value)
    if function == "count_reasoning_tokens":
        return reasoning_case(input_value)
    if function == "capture_cache_creation":
        return capture_case(input_value)
    if function == "attach_cache_creation":
        return attach_case(input_value)
    if function == "apply_grounding":
        return grounding_case(input_value)
    raise ValueError(f"unknown stream_usage_assembly function: {function}")
