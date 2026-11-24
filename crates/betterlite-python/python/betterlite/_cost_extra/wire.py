import json
from collections.abc import Callable
from typing import Final, TypeAlias, TypeVar

from pydantic import JsonValue, TypeAdapter

from betterlite._cost.engine import SETTINGS, native_call
from betterlite._cost.wire import COST_PAIR, BreakdownWire
from betterlite._native import NativeCostExtra
from betterlite.types.utils.usage import Usage

JsonObject: TypeAlias = dict[str, JsonValue]

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
def native_answer(
    adapter: TypeAdapter[T], method: Callable[[str], str], request: JsonObject
) -> T:
    text = native_call(method, json.dumps(request))
    return adapter.validate_json(text)


def pair(method: Callable[[str], str], request: JsonObject) -> tuple[float, float]:
    answer = native_answer(COST_PAIR, method, request)
    return (answer[0], answer[1])


# LiteLLM sums every numeric Usage attribute besides its counters, so extras travel flat by name.
def numeric_extras(usage: Usage) -> JsonObject:
    dumped: dict[str, JsonValue] = usage.model_dump()
    extras: JsonObject = {}
    for name, value in dumped.items():
        # ast-grep-ignore: no-isinstance
        if name not in COUNTERS and isinstance(value, int | float):
            extras[name] = value
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
