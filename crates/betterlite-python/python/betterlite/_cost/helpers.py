import time
from collections.abc import Mapping
from typing import Final

from pydantic import JsonValue

from betterlite._cost.objects import dump_object
from betterlite._cost.wire import JsonSource, number_value, object_value

# litellm.DEFAULT_REPLICATE_GPU_PRICE_PER_SECOND: an A100 80GB second, in dollars.
REPLICATE_PRICE_PER_SECOND: Final = 0.001400
COST_HEADER: Final = "llm_provider-x-litellm-response-cost"


def get_replicate_completion_pricing(  # ast-grep-ignore: no-long-names
    completion_response: JsonSource,
    total_time: float = 0.0,
) -> float:
    if total_time == 0.0:
        dumped = dump_object(completion_response)
        started_at = number_value(dumped.get("created"))
        ended_at = number_value(dumped.get("ended"))
        created = time.time() if started_at is None else started_at
        ended = time.time() if ended_at is None else ended_at
        total_time = ended - created
    return REPLICATE_PRICE_PER_SECOND * total_time / 1000


def get_response_cost_from_hidden_params(  # ast-grep-ignore: no-long-names
    hidden_params_object: JsonSource,
) -> float | None:
    dumped = dump_object(hidden_params_object)
    headers = object_value(dumped.get("additional_headers"))
    if headers is None:
        return None
    return header_cost(headers)


def header_cost(headers: Mapping[str, JsonValue]) -> float | None:
    if COST_HEADER not in headers:
        return None
    cost = headers[COST_HEADER]
    if cost is None:
        return None
    match cost:
        case bool() | int() | float() | str():
            return float(cost)
        case list() | Mapping():
            raise TypeError("the provider response-cost header is not a number")
