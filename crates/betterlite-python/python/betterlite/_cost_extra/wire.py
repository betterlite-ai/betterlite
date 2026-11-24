import json
from collections.abc import Callable
from typing import Final, TypeAlias

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite._cost.engine import SETTINGS, native_call
from betterlite._cost.wire import COST_PAIR, JSON_OBJECT, BreakdownWire
from betterlite._native import NativeCostExtra
from betterlite.types.utils.usage import Usage

JsonObject: TypeAlias = dict[str, JsonValue]

# The same settings text NativeCost reads; this family shares nothing else with it.
ENGINE: Final = NativeCostExtra(json.dumps(SETTINGS))


class CostAnswerWire(TypedDict):
    cost: float


class RealtimeCostAnswerWire(TypedDict):
    cost: float
    breakdown: BreakdownWire | None


COST: Final[TypeAdapter[CostAnswerWire]] = TypeAdapter(CostAnswerWire)
REALTIME_COST: Final[TypeAdapter[RealtimeCostAnswerWire]] = TypeAdapter(RealtimeCostAnswerWire)
USAGE: Final[TypeAdapter[Usage]] = TypeAdapter(Usage)
USAGES: Final[TypeAdapter[list[Usage]]] = TypeAdapter(list[Usage])
PARTITION: Final[TypeAdapter[list[tuple[str | None, list[JsonObject]]]]] = TypeAdapter(
    list[tuple[str | None, list[JsonObject]]]
)
COUNTERS: Final[frozenset[str]] = frozenset(
    {"prompt_tokens", "completion_tokens", "total_tokens"}
)


def pair(method: Callable[[str], str], request: JsonObject) -> tuple[float, float]:
    answer = COST_PAIR.validate_json(native_call(method, json.dumps(request)))
    return (answer[0], answer[1])


def cost_value(method: Callable[[str], str], request: JsonObject) -> float:
    answer = COST.validate_json(native_call(method, json.dumps(request)))
    return answer["cost"]


def usage_value(method: Callable[[str], str], request: JsonObject) -> Usage:
    return USAGE.validate_json(
        native_call(method, json.dumps(request))
    )


def usages_value(method: Callable[[str], str], request: JsonObject) -> list[Usage]:
    return USAGES.validate_json(
        native_call(method, json.dumps(request))
    )


# LiteLLM sums every numeric Usage attribute besides its counters, so the extras go flat to Rust.
def numeric_extras(usage: Usage) -> dict[str, int | float]:
    return {
        name: value
        for name, value in usage.model_dump().items()
        # ast-grep-ignore: no-isinstance
        if name not in COUNTERS and isinstance(value, int | float)
    }


def usage_wire(usage: Usage) -> JsonObject:
    details = usage.prompt_tokens_details
    completion = usage.completion_tokens_details
    return JSON_OBJECT.validate_python(
        {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
            "prompt_tokens_details": None if details is None else details.model_dump(),
            "completion_tokens_details": None if completion is None else completion.model_dump(),
            "numeric_extras": numeric_extras(usage),
        }
    )
