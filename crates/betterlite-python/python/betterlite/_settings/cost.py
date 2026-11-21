import json

from pydantic import JsonValue

from betterlite import _native

model_cost: dict[str, JsonValue] = json.loads(_native.bundled_model_cost_json())
model_cost.pop("fallback_generalizations", None)
cost_discount_config: dict[str, float] = {}
cost_margin_config: dict[str, float | dict[str, float]] = {}
custom_prompt_dict: dict[str, dict[str, JsonValue]] = {}


__all__ = [
    "cost_discount_config", "cost_margin_config", "custom_prompt_dict", "model_cost",
]
