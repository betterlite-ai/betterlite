from decimal import Decimal
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm import cost_calculator

if TYPE_CHECKING:
    from collections.abc import Callable

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _decimal_text(value: float) -> str:
    return format(Decimal(repr(value)).normalize(), "f")


def run(input_value: dict[str, Json]) -> Json:
    _ = litellm.__name__
    custom = input_value.get("custom_cost_per_token")
    custom_rates = cast("dict[str, float] | None", custom)
    helper_name = "_cost_per_token_custom_pricing_helper"
    calculator = cast(
        "Callable[..., tuple[float, float]]",
        getattr(cost_calculator, helper_name),
    )
    result = calculator(
        prompt_tokens=float(cast("int | float", input_value.get("prompt_tokens", 0))),
        completion_tokens=float(cast("int | float", input_value.get("completion_tokens", 0))),
        cached_tokens=float(cast("int | float", input_value.get("cached_tokens", 0))),
        cache_creation_tokens=float(
            cast("int | float", input_value.get("cache_creation_tokens", 0))
        ),
        response_time_ms=cast("float | None", input_value.get("response_time_ms", 0.0)),
        custom_cost_per_token=custom_rates,
        custom_cost_per_second=cast("float | None", input_value.get("custom_cost_per_second")),
    )
    return {"input_cost": _decimal_text(result[0]), "output_cost": _decimal_text(result[1])}
