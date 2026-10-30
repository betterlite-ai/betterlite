from datetime import datetime
from decimal import Decimal
from typing import TypeAlias, cast

from litellm.litellm_core_utils.llm_cost_calc.utils import (
    TokenRates,
    apply_off_peak_pricing,
)
from litellm.litellm_core_utils.llm_cost_calc.utils import (
    _apply_off_peak_to_base_costs as apply_off_peak_to_base_costs,  # pyright: ignore[reportPrivateUsage]
)
from litellm.types.utils import ModelInfo, OffPeakPricing

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def parse_token_rates(value: dict[str, Json]) -> TokenRates:
    reasoning_rate = value.get("reasoning_rate")
    return TokenRates(
        input_rate=float(cast(float, value["input_rate"])),
        output_rate=float(cast(float, value["output_rate"])),
        cache_read_rate=float(cast(float, value["cache_read_rate"])),
        cache_creation_rate=float(cast(float, value["cache_creation_rate"])),
        reasoning_rate=None if reasoning_rate is None else float(cast(float, reasoning_rate)),
    )


def _decimal_str(value: float) -> str:
    """Render a float as the plain decimal string betterlite's Decimal rates serialize to.

    No arithmetic touches these rates between parsing and this call, so repr's shortest
    round-trip text still carries the exact digits of the original JSON literal.
    """
    return format(Decimal(repr(value)).normalize(), "f")


def token_rates_json(rates: TokenRates) -> Json:
    return {
        "input_rate": _decimal_str(rates.input_rate),
        "output_rate": _decimal_str(rates.output_rate),
        "cache_read_rate": _decimal_str(rates.cache_read_rate),
        "cache_creation_rate": _decimal_str(rates.cache_creation_rate),
        "reasoning_rate": (
            None if rates.reasoning_rate is None else _decimal_str(rates.reasoning_rate)
        ),
        "billed_reasoning_rate": _decimal_str(rates.billed_reasoning_rate),
    }


def run(input_value: dict[str, Json]) -> Json:
    off_peak_pricing = input_value.get("off_peak_pricing")
    model_info = cast(
        ModelInfo, {"off_peak_pricing": cast("OffPeakPricing | None", off_peak_pricing)}
    )
    moment = datetime.fromisoformat(str(input_value["moment"]))

    rates = input_value["rates"]
    if not isinstance(rates, dict):
        raise TypeError("rates must be an object")
    resolved_rates = apply_off_peak_pricing(model_info, moment, parse_token_rates(rates))

    base_costs = input_value["base_costs"]
    if not isinstance(base_costs, dict):
        raise TypeError("base_costs must be an object")
    above_1hr_rate = base_costs.get("cache_creation_above_1hr_rate")
    resolved_base_costs = apply_off_peak_to_base_costs(
        model_info,
        moment,
        (
            float(cast(float, base_costs["prompt_rate"])),
            float(cast(float, base_costs["completion_rate"])),
            float(cast(float, base_costs["cache_creation_rate"])),
            None if above_1hr_rate is None else float(cast(float, above_1hr_rate)),
            float(cast(float, base_costs["cache_read_rate"])),
        ),
    )

    return {
        "rates": token_rates_json(resolved_rates),
        "base_costs": {
            "prompt_rate": _decimal_str(resolved_base_costs[0]),
            "completion_rate": _decimal_str(resolved_base_costs[1]),
            "cache_creation_rate": _decimal_str(resolved_base_costs[2]),
            "cache_creation_above_1hr_rate": _decimal_str(resolved_base_costs[3]),
            "cache_read_rate": _decimal_str(resolved_base_costs[4]),
        },
    }
