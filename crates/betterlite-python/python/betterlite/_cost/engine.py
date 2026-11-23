import json
from collections.abc import Callable
from typing import Final

from betterlite._completion.errors import raise_native_error
from betterlite._cost.logging import CostLogging, store_breakdown
from betterlite._cost.wire import (
    COST_ANSWER,
    COST_PAIR,
    USAGE_ANSWER,
    CallRequestWire,
    CompletionRequestWire,
    UsageRequestWire,
)
from betterlite._native import NativeCost, NativeError
from betterlite._settings.cost import cost_discount_config, cost_margin_config, model_cost
from betterlite.types.utils.usage import Usage

# The pricing settings the native engine reads once; the catalog is the model_cost map LiteLLM prices from.
SETTINGS: Final = {
    "model_cost": model_cost,
    "cost_discount_config": cost_discount_config,
    "cost_margin_config": cost_margin_config,
}
ENGINE: Final = NativeCost(json.dumps(SETTINGS))


def native_call(method: Callable[[str], str], text: str) -> str:
    try:
        return method(text)
    except NativeError as error:
        raise_native_error(error)


def price_pair(request: CallRequestWire) -> tuple[float, float]:
    answer = COST_PAIR.validate_json(native_call(ENGINE.cost_per_token, json.dumps(request)))
    return (answer[0], answer[1])


# completion_cost and response_cost price the same request; each keeps its own native entry point.
def charge_answer(
    request: CompletionRequestWire,
    logging_obj: CostLogging | None,
    price: Callable[[str], str],
) -> float:
    answer = COST_ANSWER.validate_json(native_call(price, json.dumps(request)))
    store_breakdown(logging_obj, answer)
    return answer["cost"]


def usage_value(request: UsageRequestWire) -> Usage | None:
    answer = USAGE_ANSWER.validate_json(native_call(ENGINE.usage_object, json.dumps(request)))
    found = answer["usage"]
    if found is None:
        return None
    return Usage(**found)
