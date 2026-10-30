from decimal import Decimal
from typing import TypeAlias, cast

from litellm.litellm_core_utils.llm_cost_calc.tiered_pricing import (
    select_tier_for_input,  # pyright: ignore[reportUnknownVariableType]
    tier_rate,  # pyright: ignore[reportUnknownVariableType]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    tiers_value = input_value.get("tiers", [])
    if not isinstance(tiers_value, list):
        raise ValueError("tiers must be a JSON array")
    tokens_value = input_value.get("tokens")
    if not isinstance(tokens_value, int) or isinstance(tokens_value, bool):
        raise ValueError("tokens must be a JSON integer")
    cost_key_value = input_value.get("cost_key")
    if not isinstance(cost_key_value, str):
        raise ValueError("cost_key must be a string")
    fallback_cost_key_value = input_value.get("fallback_cost_key")
    if fallback_cost_key_value is not None and not isinstance(fallback_cost_key_value, str):
        raise ValueError("fallback_cost_key must be a string or null")
    tiers = cast("list[dict[str, Json]]", tiers_value)
    selected = cast("dict[str, Json] | None", select_tier_for_input(tiers, tokens_value))
    if selected is None:
        return {"selected_range": None, "rate": None}
    rate = tier_rate(selected, cost_key_value, fallback_cost_key_value)
    return {
        "selected_range": selected.get("range"),
        "rate": format(Decimal(repr(rate)).normalize(), "f"),
    }
