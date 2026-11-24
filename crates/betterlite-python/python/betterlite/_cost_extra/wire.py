import json
from collections.abc import Callable, Mapping
from typing import Final, TypeAlias

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite._cost.engine import SETTINGS, native_call
from betterlite._cost.wire import BreakdownWire
from betterlite._native import NativeCostExtra
from betterlite.types.rerank import RerankBilledUnits
from betterlite.types.utils.usage import Usage
from betterlite.types.vector_stores import VectorStoreSearchResponse

JsonObject: TypeAlias = dict[str, JsonValue]

# The same settings text NativeCost reads; this family shares nothing else with it.
ENGINE: Final = NativeCostExtra(json.dumps(SETTINGS))


class OcrUsageWire(TypedDict):
    credits: float | None
    pages_processed: int | None
    pages_processed_annotation: int | None


class UsageWire(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: JsonObject | None
    completion_tokens_details: JsonObject | None
    numeric_extras: dict[str, int | float]


class OcrRequestWire(TypedDict):
    model: str
    custom_llm_provider: str | None
    usage: OcrUsageWire | None
    model_info: Mapping[str, JsonValue] | None


class OcrBatchRequestWire(TypedDict):
    model: str
    custom_llm_provider: str | None
    usage: OcrUsageWire
    model_info: Mapping[str, JsonValue] | None


class VectorStoreRequestWire(TypedDict):
    model: str | None
    custom_llm_provider: str | None
    response: VectorStoreSearchResponse


class RerankRequestWire(TypedDict):
    model: str
    custom_llm_provider: str | None
    billed_units: RerankBilledUnits | None


class TranscriptionRequestWire(TypedDict):
    model: str
    custom_llm_provider: str | None
    duration: float


class ImageRequestWire(TypedDict):
    model: str
    custom_llm_provider: str | None
    quality: str | None
    n: int | None
    size: str | None
    model_info: Mapping[str, JsonValue] | None


class VideoRequestWire(TypedDict):
    model: str
    duration_seconds: float
    custom_llm_provider: str | None
    model_info: Mapping[str, JsonValue] | None
    video_resolution: str | None


class BatchRequestWire(TypedDict):
    usage: UsageWire
    model: str
    custom_llm_provider: str | None
    model_info: Mapping[str, JsonValue] | None
    data_residency: str | None


class CombineRequestWire(TypedDict):
    usages: list[UsageWire]


class CollectRequestWire(TypedDict):
    source: str
    results: list[JsonObject]


class PartitionRequestWire(TypedDict):
    results: list[JsonObject]


class RealtimeRequestWire(TypedDict):
    results: list[JsonObject]
    combined_usage: UsageWire
    custom_llm_provider: str
    litellm_model_name: str
    data_residency: str | None
    custom_pricing_model: str | None
    base_pricing_model: str | None


class RealtimeTranscriptionRequestWire(TypedDict):
    results: list[JsonObject]
    custom_llm_provider: str
    litellm_model_name: str
    custom_pricing_model: str | None


class CostAnswerWire(TypedDict):
    cost: float


class RealtimeCostAnswerWire(TypedDict):
    cost: float
    breakdown: BreakdownWire | None


class UsageAnswerWire(TypedDict):
    usage: JsonObject


class UsagesAnswerWire(TypedDict):
    usages: list[JsonObject]


class PartitionGroupWire(TypedDict):
    service_tier: str | None
    events: list[JsonObject]


class PartitionAnswerWire(TypedDict):
    tiers: list[PartitionGroupWire]


PAIR: Final[TypeAdapter[tuple[float, float]]] = TypeAdapter(tuple[float, float])
COST: Final[TypeAdapter[CostAnswerWire]] = TypeAdapter(CostAnswerWire)
REALTIME_COST: Final[TypeAdapter[RealtimeCostAnswerWire]] = TypeAdapter(RealtimeCostAnswerWire)
USAGE: Final[TypeAdapter[UsageAnswerWire]] = TypeAdapter(UsageAnswerWire)
USAGES: Final[TypeAdapter[UsagesAnswerWire]] = TypeAdapter(UsagesAnswerWire)
PARTITION: Final[TypeAdapter[PartitionAnswerWire]] = TypeAdapter(PartitionAnswerWire)
COUNTERS: Final[frozenset[str]] = frozenset(
    {"prompt_tokens", "completion_tokens", "total_tokens"}
)


def pair(method: Callable[[str], str], text: str) -> tuple[float, float]:
    answer = PAIR.validate_json(native_call(method, text))
    return (answer[0], answer[1])


def cost(method: Callable[[str], str], text: str) -> float:
    return COST.validate_json(native_call(method, text))["cost"]


def usage_answer(method: Callable[[str], str], text: str) -> Usage:
    answer = USAGE.validate_json(native_call(method, text))
    return Usage.model_validate(answer["usage"])


def usages_answer(method: Callable[[str], str], text: str) -> list[Usage]:
    answer = USAGES.validate_json(native_call(method, text))
    return [Usage.model_validate(usage) for usage in answer["usages"]]


# LiteLLM sums every numeric attribute of a Usage besides its counters, so the extras go to Rust flat.
def numeric_extras(usage: Usage) -> dict[str, int | float]:
    return {
        name: value
        for name, value in usage.model_dump().items()
        if name not in COUNTERS and isinstance(value, int | float)  # ast-grep-ignore: no-isinstance
    }


def usage_wire(usage: Usage) -> UsageWire:
    details = usage.prompt_tokens_details
    completion = usage.completion_tokens_details
    return UsageWire(
        prompt_tokens=usage.prompt_tokens,
        completion_tokens=usage.completion_tokens,
        total_tokens=usage.total_tokens,
        prompt_tokens_details=None if details is None else details.model_dump(),
        completion_tokens_details=None if completion is None else completion.model_dump(),
        numeric_extras=numeric_extras(usage),
    )
