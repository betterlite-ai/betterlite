from collections.abc import Mapping, Sequence
from typing import Final, TypeAlias

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite.types.utils.base import Dumpable, InitValue

JsonSource: TypeAlias = Dumpable | Mapping[str, JsonValue]

class CallRequestWire(TypedDict, total=False):
    model: str
    prompt_tokens: int
    completion_tokens: int
    response_time_ms: float | None
    custom_llm_provider: str | None
    region_name: str | None
    prompt_characters: int | None
    completion_characters: int | None
    cache_creation_input_tokens: int | None
    cache_read_input_tokens: int | None
    custom_cost_per_token: dict[str, JsonValue] | None
    custom_cost_per_second: float | None
    number_of_queries: int | None
    usage: dict[str, JsonValue] | None
    rerank_billed_units: dict[str, JsonValue] | None
    call_type: str
    audio_seconds: float
    service_tier: str | None
    data_residency: str | None
    vertex_location: str | None
    request_model: str | None

class ResponseWire(TypedDict, total=False):
    kind: str
    model: str | None
    usage: dict[str, JsonValue] | None
    service_tier: str | None
    hidden_params: dict[str, JsonValue] | None
    created: float | None
    ended: float | None

class CompletionRequestWire(TypedDict, total=False):
    model: str | None
    prompt: str
    messages: list[dict[str, JsonValue]]
    completion: str
    total_time: float | None
    call_type: str | None
    custom_llm_provider: str | None
    region_name: str | None
    custom_cost_per_token: dict[str, JsonValue] | None
    custom_cost_per_second: float | None
    optional_params: dict[str, JsonValue] | None
    custom_pricing: bool | None
    base_model: str | None
    router_model_id: str | None
    service_tier: str | None
    data_residency: str | None
    vertex_location: str | None
    response: ResponseWire | None

class BreakdownWire(TypedDict, total=False):
    input_cost: float
    cache_read_cost: float
    cache_creation_cost: float
    output_cost: float
    reasoning_cost: float
    total_cost: float
    tool_usage_cost: float
    guardrail_cost: float
    additional_costs: dict[str, float]
    original_cost: float
    discount_percent: float
    discount_amount: float
    margin_percent: float
    margin_fixed_amount: float
    margin_total_amount: float
    service_tier: str
    data_residency: str
    vertex_location: str

class RatesWire(TypedDict):
    input_cost_per_token: float
    output_cost_per_token: float
    cache_read_input_token_cost: float
    cache_read_input_audio_token_cost: float  # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost: float  # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost_above_1hr: float  # ast-grep-ignore: no-long-names
    output_cost_per_reasoning_token: float  # ast-grep-ignore: no-long-names

class CostAnswerWire(TypedDict, total=False):
    cost: float
    breakdown: BreakdownWire | None
    billed_token_rates: RatesWire | None

class UsageRequestWire(TypedDict, total=False):
    kind: str
    usage: dict[str, JsonValue] | None

class UsageAnswerWire(TypedDict):
    usage: dict[str, JsonValue] | None

COST_PAIR: Final[TypeAdapter[list[float]]] = TypeAdapter(list[float])
COST_ANSWER: Final[TypeAdapter[CostAnswerWire]] = TypeAdapter(CostAnswerWire)
USAGE_ANSWER: Final[TypeAdapter[UsageAnswerWire]] = TypeAdapter(UsageAnswerWire)
JSON_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])

def text_value(value: JsonValue) -> str | None:
    match value:
        case str():
            return value
        case int() | float() | bool() | None | list() | Mapping():
            return None

def object_value(value: InitValue) -> dict[str, JsonValue] | None:
    match value:
        case Dumpable():
            return value.model_dump()
        case Mapping():
            return JSON_OBJECT.validate_python(value)
        case str() | int() | float() | bool() | None | Sequence():
            return None

def number_value(value: JsonValue) -> float | None:
    match value:
        case bool() | int() | float() | str():
            return float(value)
        case None:
            return None
        case list() | Mapping():
            raise TypeError("a response timestamp must be a number")
