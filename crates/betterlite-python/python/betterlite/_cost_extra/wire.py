import json
from collections.abc import Callable
from typing import Final, TypeAlias, TypeVar

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite._cost.engine import SETTINGS, native_call
from betterlite._cost.wire import COST_PAIR, BreakdownWire
from betterlite._native import NativeCostExtra
from betterlite.types.utils.usage import Usage

JsonObject: TypeAlias = dict[str, JsonValue]

class OcrUsageWire(TypedDict, total=False):
    credits: float | None
    pages_processed: int | None
    pages_processed_annotation: int | None
class OcrRequestWire(TypedDict, total=False):
    model: str
    custom_llm_provider: str | None
    usage: OcrUsageWire
    model_info: dict[str, JsonValue] | None
class VectorStoreRequestWire(TypedDict, total=False):
    model: str | None
    custom_llm_provider: str | None
    response: dict[str, JsonValue]
class RerankRequestWire(TypedDict, total=False):
    model: str
    custom_llm_provider: str | None
    billed_units: dict[str, JsonValue]
class TranscriptionRequestWire(TypedDict, total=False):
    model: str
    custom_llm_provider: str | None
    duration: float
class ImageRequestWire(TypedDict, total=False):
    model: str
    custom_llm_provider: str | None
    quality: str | None
    n: int | None
    size: str | None
    model_info: dict[str, JsonValue] | None
class VideoRequestWire(TypedDict, total=False):
    model: str
    duration_seconds: float
    custom_llm_provider: str | None
    model_info: dict[str, JsonValue] | None
    video_resolution: str
class BatchRequestWire(TypedDict, total=False):
    usage: JsonObject
    model: str
    custom_llm_provider: str | None
    model_info: dict[str, JsonValue] | None
    data_residency: str | None
class CombineUsageRequestWire(TypedDict, total=False):
    usages: list[JsonObject]
class CollectUsageRequestWire(TypedDict, total=False):
    source: str
    results: list[dict[str, JsonValue]]
class PartitionUsageRequestWire(TypedDict, total=False):
    results: list[dict[str, JsonValue]]
class RealtimeCostRequestWire(TypedDict, total=False):
    results: list[dict[str, JsonValue]]
    combined_usage: JsonObject
    custom_llm_provider: str
    litellm_model_name: str
    data_residency: str | None
    custom_pricing_model: str | None
    base_pricing_model: str | None
class RealtimeTranscriptionWire(TypedDict, total=False):
    results: list[dict[str, JsonValue]]
    custom_llm_provider: str
    litellm_model_name: str
    custom_pricing_model: str | None

# The same settings text NativeCost reads; this family shares nothing else with it.
ENGINE: Final = NativeCostExtra(json.dumps(SETTINGS))

REALTIME_COST: Final[TypeAdapter[tuple[float, BreakdownWire | None]]] = TypeAdapter(
    tuple[float, BreakdownWire | None]
)
FLOAT: Final[TypeAdapter[float]] = TypeAdapter(float)
USAGE: Final[TypeAdapter[Usage]] = TypeAdapter(Usage)
USAGES: Final[TypeAdapter[list[Usage]]] = TypeAdapter(list[Usage])
PARTITION: Final[TypeAdapter[list[tuple[str | None, list[JsonObject]]]]] = TypeAdapter(
    list[tuple[str | None, list[JsonObject]]]
)
COUNTERS: Final[frozenset[str]] = frozenset(("prompt_tokens", "completion_tokens", "total_tokens"))

T = TypeVar("T")


# Every call hands the engine JSON text; the adapter turns the answer back into its Python type.
def native_answer(adapter: TypeAdapter[T], method: Callable[[str], str], request: str) -> T:
    text = native_call(method, request)
    return adapter.validate_json(text)


def pair(method: Callable[[str], str], request: str) -> tuple[float, float]:
    answer = native_answer(COST_PAIR, method, request)
    return (answer[0], answer[1])


# LiteLLM sums every numeric Usage attribute besides its counters, so extras travel flat by name.
def numeric_extras(usage: Usage) -> JsonObject:
    dumped: dict[str, JsonValue] = usage.model_dump()
    extras: JsonObject = {}
    for name, value in dumped.items():
        match value:
            case bool() | int() | float():
                if name not in COUNTERS:
                    extras[name] = value
            case str() | None | list() | dict():
                continue
    return extras


def usage_wire(usage: Usage) -> JsonObject:
    details = usage.prompt_tokens_details
    completion = usage.completion_tokens_details
    fields: JsonObject = {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_tokens_details": None if details is None else details.model_dump(),
        "completion_tokens_details": None if completion is None else completion.model_dump(),
    }
    fields.update(numeric_extras(usage))
    return fields
