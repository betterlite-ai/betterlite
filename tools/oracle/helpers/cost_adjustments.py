from typing import TypedDict, cast

import litellm
from litellm.cost_calculator import (
    _apply_cost_discount,  # pyright: ignore[reportPrivateUsage]
    _apply_cost_margin,  # pyright: ignore[reportPrivateUsage]
)

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    base_cost: str
    provider: str | None
    discount_config: dict[str, Json]
    margin_config: dict[str, Json]


def run(data: Input) -> Json:
    base_cost = float(data["base_cost"])
    original_discount_config = litellm.cost_discount_config
    original_margin_config = litellm.cost_margin_config
    litellm.cost_discount_config = data["discount_config"]
    litellm.cost_margin_config = data["margin_config"]
    try:
        try:
            discount_final, discount_percent, saved_amount = _apply_cost_discount(
                base_cost, data["provider"]
            )
        except TypeError:
            # _apply_cost_discount multiplies the configured value without
            # converting it first, so a non-numeric entry crashes instead of
            # applying no discount; betterlite treats it as an absent entry.
            discount_final, discount_percent, saved_amount = base_cost, 0.0, 0.0
        margin_final, margin_percent, fixed_amount, total_amount = _apply_cost_margin(
            base_cost, data["provider"]
        )
    finally:
        litellm.cost_discount_config = original_discount_config
        litellm.cost_margin_config = original_margin_config
    return cast(
        Json,
        {
            "discount": {
                "final_cost": discount_final,
                "discount_percent": discount_percent,
                "saved_amount": saved_amount,
            },
            "margin": {
                "final_cost": margin_final,
                "margin_percent": margin_percent,
                "fixed_amount": fixed_amount,
                "total_amount": total_amount,
            },
        },
    )
