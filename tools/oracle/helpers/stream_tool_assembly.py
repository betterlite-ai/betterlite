import json
from types import SimpleNamespace
from typing import Any, TypeAlias, cast

from litellm.litellm_core_utils.streaming_chunk_builder_utils import ChunkProcessor

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def namespace(value: Json) -> object:
    if not isinstance(value, dict):
        return value
    return SimpleNamespace(**{key: namespace(item) for key, item in value.items()})


def object_chunks(chunks: list[dict[str, Json]]) -> list[dict[str, Json]]:
    converted: list[dict[str, Json]] = []
    for chunk in chunks:
        choices = cast(list[dict[str, Json]], chunk.get("choices", []))
        converted.append({"choices": [object_choice(choice) for choice in choices]})
    return converted


def object_choice(choice: dict[str, Json]) -> dict[str, Json]:
    delta = cast(dict[str, Json], choice.get("delta", {}))
    tools = cast(list[Json], delta.get("tool_calls", []))
    result = {**choice, "delta": {**delta, "tool_calls": [namespace(tool) for tool in tools]}}
    return cast(dict[str, Json], result)


def function_chunks(chunks: list[dict[str, Json]]) -> list[dict[str, Json]]:
    converted: list[dict[str, Json]] = []
    for chunk in chunks:
        choices = cast(list[dict[str, Json]], chunk.get("choices", []))
        converted.append({"choices": [function_choice(choice) for choice in choices]})
    return converted


def function_choice(choice: dict[str, Json]) -> dict[str, Json]:
    delta = cast(dict[str, Json], choice.get("delta", {}))
    call = cast(dict[str, Json], delta["function_call"]) if "function_call" in delta else None
    result = {
        **choice,
        "delta": {**delta, "function_call": namespace(call) if call is not None else None},
    }
    return cast(dict[str, Json], result)


def json_result(result: object) -> Json:
    if isinstance(result, list):
        payload = [cast(Any, item).model_dump() for item in result]  # pyright: ignore[reportAny, reportUnknownVariableType]
        return cast(Json, json.loads(json.dumps(payload)))
    encoded = cast(str, cast(Any, result).model_dump_json())  # pyright: ignore[reportAny]
    return cast(Json, json.loads(encoded))


def run(input_value: dict[str, Json]) -> Json:
    if "tool" in input_value:
        arguments = cast(dict[str, Json], input_value["tool"])
        chunks = cast(list[dict[str, Json]], arguments["chunks"])
        if arguments.get("object_form") is True:
            chunks = object_chunks(chunks)
        result = cast(Any, ChunkProcessor(chunks)).get_combined_tool_content(chunks)  # pyright: ignore[reportAny]
    else:
        arguments = cast(dict[str, Json], input_value["function"])
        chunks = function_chunks(cast(list[dict[str, Json]], arguments["chunks"]))
        result = cast(Any, ChunkProcessor(chunks)).get_combined_function_call_content(chunks)  # pyright: ignore[reportAny]
    return json_result(cast(object, result))
