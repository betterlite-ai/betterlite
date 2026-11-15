import asyncio
from decimal import Decimal
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.batches.batch_utils import (
    BatchCostUsageResult,
    calculate_batch_cost_and_usage,
    calculate_vertex_ai_batch_cost_and_usage,
)
from litellm.types.utils import ModelInfo, Usage
from pydantic import JsonValue, TypeAdapter, ValidationError

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

Provider: TypeAlias = Literal["openai", "azure", "vertex_ai", "hosted_vllm", "anthropic", "mistral"]


class BytesContent(TypedDict):
    kind: Literal["bytes"]
    bytes: list[int]


class LinesContent(TypedDict):
    kind: Literal["lines"]
    lines: list[JsonValue]


Content: TypeAlias = BytesContent | LinesContent


class OutputUsageInput(TypedDict):
    kind: Literal["output_usage"]
    provider: Provider
    model_name: str | None
    model_info: ModelInfo | None
    content: Content


class VertexUsageInput(TypedDict):
    kind: Literal["vertex_usage"]
    model_name: str | None
    model_info: ModelInfo | None
    content: Content


Input: TypeAlias = OutputUsageInput | VertexUsageInput


def _decimal_str(value: float) -> str:
    # Cases keep every computed value to a short exact product, so repr carries the decimal digits.
    normalized = Decimal(repr(value)).normalize()
    return format(normalized, "f")


def _parse_bytes(raw: bytes) -> tuple[list[dict[str, JsonValue]], list[JsonValue]]:
    entries: list[dict[str, JsonValue]] = []
    diagnostics: list[JsonValue] = []
    for line in raw.split(b"\n"):
        stripped = line.strip()
        if not stripped:
            continue
        try:
            parsed = JSON.validate_json(stripped)
        except ValidationError:
            diagnostics.append({"kind": "malformed_line"})
            continue
        match parsed:
            case dict():
                entries.append(parsed)
            case None | bool() | int() | float() | str() | list():
                diagnostics.append({"kind": "non_object_line", "type": type(parsed).__name__})
    return entries, diagnostics


def _line_entries(lines: list[JsonValue]) -> tuple[list[dict[str, JsonValue]], list[JsonValue]]:
    entries: list[dict[str, JsonValue]] = []
    diagnostics: list[JsonValue] = []
    for line in lines:
        match line:
            case dict():
                entries.append(line)
            case None | bool() | int() | float() | str() | list():
                diagnostics.append({"kind": "non_object_line", "type": type(line).__name__})
    return entries, diagnostics


def _result_value(result: BatchCostUsageResult, diagnostics: list[JsonValue]) -> JsonValue:
    prompt_details = result.usage.prompt_tokens_details
    completion_details = result.usage.completion_tokens_details
    reasoning_tokens = 0
    if completion_details is not None and completion_details.reasoning_tokens is not None:
        reasoning_tokens = completion_details.reasoning_tokens
    cache_read_tokens = 0
    if prompt_details is not None and prompt_details.cached_tokens is not None:
        cache_read_tokens = prompt_details.cached_tokens
    cache_creation_tokens = 0
    if prompt_details is not None and prompt_details.cache_write_tokens is not None:
        cache_creation_tokens = prompt_details.cache_write_tokens
    return JSON.validate_python(
        {
            "cost": _decimal_str(result.cost),
            "prompt_cost": _decimal_str(result.prompt_cost),
            "completion_cost": _decimal_str(result.completion_cost),
            "usage": {
                "prompt_tokens": result.usage.prompt_tokens,
                "completion_tokens": result.usage.completion_tokens,
                "total_tokens": result.usage.total_tokens,
                "reasoning_tokens": reasoning_tokens,
                "cache_read_tokens": cache_read_tokens,
                "cache_creation_tokens": cache_creation_tokens,
            },
            "models": result.models,
            "successful_requests": result.successful_requests,
            "failed_requests": result.failed_requests,
            "diagnostics": diagnostics,
        }
    )


def run(input_value: Input) -> JsonValue:
    content = input_value["content"]
    if content["kind"] == "bytes":
        entries, diagnostics = _parse_bytes(bytes(content["bytes"]))
    else:
        entries, diagnostics = _line_entries(content["lines"])
    if input_value["kind"] == "vertex_usage":
        result = calculate_vertex_ai_batch_cost_and_usage(
            entries, input_value["model_name"], model_info=input_value["model_info"]
        )
    else:
        result = asyncio.run(
            calculate_batch_cost_and_usage(
                entries,
                input_value["provider"],
                input_value["model_name"],
                model_info=input_value["model_info"],
            )
        )
    return _result_value(result, diagnostics)
