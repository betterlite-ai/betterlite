import os
from collections.abc import Callable
from typing import Literal, NotRequired, TypedDict

import litellm
from litellm.llms.bedrock.common_utils import (
    bedrock_converse_supports_parallel_tool_use_config as parallel_tool_gate,
)
from litellm.llms.bedrock.common_utils import (
    bedrock_converse_supports_strict_tools,
    bedrock_model_accepts_cache_points,
    bedrock_supports_openai_responses,
    bedrock_supports_tool_search,
    build_mantle_messages_url,
    is_claude_4_5_on_bedrock,
    normalize_bedrock_opus_output_config_effort,
)
from pydantic import JsonValue

# the flag gates above a try that turns a malformed entry into a raised marker
FLAG_LOOKUPS: dict[str, Callable[[str], bool]] = {
    "parallel_tools": parallel_tool_gate,
    "extended_cache_ttl": is_claude_4_5_on_bedrock,
}


class MantleCase(TypedDict):
    api_base: NotRequired[str]
    aws_bedrock_runtime_endpoint: NotRequired[str]
    environment: dict[str, str]


class EffortCase(TypedDict):
    model: str
    output_config: JsonValue


class MantleApiBase(TypedDict):
    function: Literal["mantle_api_base"]
    region: str
    cases: list[MantleCase]


class NativeResponses(TypedDict):
    function: Literal["native_responses"]
    model_cost: dict[str, JsonValue]
    models: list[str | None]


class CachePointUnknownModel(TypedDict):
    function: Literal["cache_point_unknown_model"]
    model_cost: dict[str, JsonValue]
    models: list[str | None]


class StrictToolsFallback(TypedDict):
    function: Literal["strict_tools_fallback"]
    model_cost: dict[str, JsonValue]
    models: list[str]


class ToolSearch(TypedDict):
    function: Literal["tool_search"]
    model_cost: dict[str, JsonValue]
    models: list[str]


class EffortCeiling(TypedDict):
    function: Literal["effort_ceiling"]
    model_cost: dict[str, JsonValue]
    cases: list[EffortCase]


class ParallelTools(TypedDict):
    function: Literal["parallel_tools"]
    model_cost: dict[str, JsonValue]
    models: list[str]


class ExtendedCacheTtl(TypedDict):
    function: Literal["extended_cache_ttl"]
    model_cost: dict[str, JsonValue]
    models: list[str]


FlagCall = CachePointUnknownModel | ParallelTools | ExtendedCacheTtl

Input = (
    MantleApiBase
    | NativeResponses
    | CachePointUnknownModel
    | StrictToolsFallback
    | ToolSearch
    | EffortCeiling
    | ParallelTools
    | ExtendedCacheTtl
)


def install_environment(environment: dict[str, str]) -> dict[str, str]:
    previous = dict(os.environ)
    os.environ.clear()
    os.environ.update(environment)
    return previous


def mantle_api_base(call: MantleApiBase) -> JsonValue:
    urls: list[JsonValue] = []
    for case in call["cases"]:
        previous = install_environment(case["environment"])
        urls.append(
            build_mantle_messages_url(
                case.get("api_base"),
                case.get("aws_bedrock_runtime_endpoint"),
                call["region"],
            )
        )
        os.environ.clear()
        os.environ.update(previous)
    return {"urls": urls}


def native_responses(call: NativeResponses) -> JsonValue:
    results: list[JsonValue] = []
    for model in call["models"]:
        try:
            results.append(bedrock_supports_openai_responses(model, call["model_cost"]))
        except TypeError:
            results.append({"raised": True})
    return {"results": results}


def model_flag(call: FlagCall) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    results: list[JsonValue] = []
    if call["function"] == "cache_point_unknown_model":
        for model in call["models"]:
            try:
                results.append(bedrock_model_accepts_cache_points(model))
            except AttributeError:
                results.append({"raised": True})
        return {"results": results}
    for model in call["models"]:
        try:
            results.append(FLAG_LOOKUPS[call["function"]](model))
        except AttributeError:
            results.append({"raised": True})
    return {"results": results}


def strict_tools(call: StrictToolsFallback) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    results: list[JsonValue] = [
        bedrock_converse_supports_strict_tools(model) for model in call["models"]
    ]
    return {"results": results}


def tool_search(call: ToolSearch) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    results: list[JsonValue] = [bedrock_supports_tool_search(model) for model in call["models"]]
    return {"results": results}


def effort_ceiling(call: EffortCeiling) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    results: list[JsonValue] = []
    for case in call["cases"]:
        output_config = case["output_config"]
        try:
            normalize_bedrock_opus_output_config_effort(case["model"], output_config)
        except TypeError:
            results.append({"raised": True})
            continue
        results.append(output_config)
    return {"results": results}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "mantle_api_base":
        return mantle_api_base(input_value)
    if input_value["function"] == "native_responses":
        return native_responses(input_value)
    if input_value["function"] == "strict_tools_fallback":
        return strict_tools(input_value)
    if input_value["function"] == "tool_search":
        return tool_search(input_value)
    if input_value["function"] == "effort_ceiling":
        return effort_ceiling(input_value)
    return model_flag(input_value)
