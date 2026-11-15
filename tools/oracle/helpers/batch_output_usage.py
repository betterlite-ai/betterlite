import asyncio
from decimal import Decimal
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.batches import batch_utils
from litellm.batches.batch_utils import BatchCostUsageResult
from pydantic import JsonValue, TypeAdapter, ValidationError

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class BytesContent(TypedDict):
    kind: Literal["bytes"]
    bytes: list[int]


class LinesContent(TypedDict):
    kind: Literal["lines"]
    lines: list[JsonValue]


Content: TypeAlias = BytesContent | LinesContent
Provider: TypeAlias = Literal["openai", "azure", "vertex_ai", "hosted_vllm", "anthropic", "mistral"]


class OutputUsageInput(TypedDict):
    kind: Literal["output_usage"]
    provider: Provider
    model_name: str | None
    model_info: dict[str, JsonValue] | None
    content: Content


class VertexUsageInput(TypedDict):
    kind: Literal["vertex_usage"]
    model_name: str | None
    model_info: dict[str, JsonValue] | None
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


def _result_value(result: BatchCostUsageResult, diagnostics: list[JsonValue]) -> JsonValue:
    prompt_details = result.usage.prompt_tokens_details
    completion_details = result.usage.completion_tokens_details
    reasoning_tokens = None if completion_details is None else completion_details.reasoning_tokens
    cached_tokens = None if prompt_details is None else prompt_details.cached_tokens
    cache_write_tokens = None if prompt_details is None else prompt_details.cache_write_tokens
    usage: dict[str, JsonValue] = {
        "prompt_tokens": result.usage.prompt_tokens,
        "completion_tokens": result.usage.completion_tokens,
        "total_tokens": result.usage.total_tokens,
        "reasoning_tokens": 0 if reasoning_tokens is None else reasoning_tokens,
        "cache_read_tokens": 0 if cached_tokens is None else cached_tokens,
        "cache_creation_tokens": 0 if cache_write_tokens is None else cache_write_tokens,
    }
    payload: dict[str, JsonValue] = {
        "cost": _decimal_str(result.cost),
        "prompt_cost": _decimal_str(result.prompt_cost),
        "completion_cost": _decimal_str(result.completion_cost),
        "usage": usage,
        "models": list(result.models),
        "successful_requests": result.successful_requests,
        "failed_requests": result.failed_requests,
        "diagnostics": diagnostics,
    }
    return JSON.validate_python(payload)


def run(input_value: Input) -> JsonValue:
    content = input_value["content"]
    if content["kind"] == "bytes":
        entries, diagnostics = _parse_bytes(bytes(content["bytes"]))
    else:
        joined = b"\n".join(JSON.dump_json(line) for line in content["lines"])
        entries, diagnostics = _parse_bytes(joined)
    # A vertex_ai batch whose first row carries a request object takes LiteLLM's native Vertex path.
    provider: Provider = "vertex_ai"
    if input_value["kind"] == "output_usage":
        provider = input_value["provider"]
    name = input_value["model_name"]
    info = input_value["model_info"]
    result = asyncio.run(batch_utils.calculate_batch_cost_and_usage(entries, provider, name, info))  # type: ignore[misc, arg-type]  # noqa: E501  # pyright: ignore[reportUnknownMemberType, reportArgumentType]  # bare dict entries and a deployment model_info LiteLLM types as a full ModelInfo: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/batches/batch_utils.py#L71-L76
    return _result_value(result, diagnostics)
