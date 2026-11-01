from collections.abc import Mapping
from decimal import Decimal
from typing import Final, TypeAlias, cast

from litellm.litellm_core_utils.llm_cost_calc.guardrail_cost import (
    azure_prompt_shield_guardrail_cost,
    billed_guardrail_cost_by_unit,
    cost_breakdown_with_guardrail,
    guardrail_cost_total,
    guardrail_information_cost,
)
from litellm.types.utils import CostBreakdown

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_STRING_BREAKDOWN_FIELDS: Final = ("service_tier", "data_residency", "vertex_location")
_DECIMAL_BREAKDOWN_FIELDS: Final = (
    "input_cost",
    "cache_read_cost",
    "cache_creation_cost",
    "output_cost",
    "reasoning_cost",
    "total_cost",
    "tool_usage_cost",
    "guardrail_cost",
    "original_cost",
    "discount_percent",
    "discount_amount",
    "margin_percent",
    "margin_fixed_amount",
    "margin_total_amount",
)
_KNOWN_BREAKDOWN_FIELDS: Final = frozenset(
    (*_STRING_BREAKDOWN_FIELDS, *_DECIMAL_BREAKDOWN_FIELDS, "additional_costs")
)


def _decimal_str(value: float) -> str:
    # No arithmetic runs before this, so repr still carries the JSON literal's digits.
    return format(Decimal(repr(value)).normalize(), "f")


def _entries_of(guardrail_information: Json) -> list[Json]:
    if guardrail_information is None:
        return []
    if isinstance(guardrail_information, list):
        return guardrail_information
    return [guardrail_information]


def _cost_by_unit_json(cost_by_unit: Mapping[str, float | None] | None) -> Json:
    if cost_by_unit is None:
        return None
    return {
        counter: None if cost is None else _decimal_str(cost)
        for counter, cost in cost_by_unit.items()
    }


def _rollup_entry(entry: Json) -> Json:
    cost_by_unit = billed_guardrail_cost_by_unit(entry)
    total = guardrail_cost_total(cost_by_unit)
    return {"cost_by_unit": _cost_by_unit_json(cost_by_unit), "total": _decimal_str(total)}


def _azure_scenario_cost(scenario: dict[str, Json]) -> Json:
    usage_units_value = scenario.get("usage_units", {})
    if not isinstance(usage_units_value, dict):
        raise ValueError("usage_units must be an object")
    usage_units = {counter: int(cast(int, units)) for counter, units in usage_units_value.items()}
    cost_tier = scenario.get("cost_tier")
    if cost_tier is not None and not isinstance(cost_tier, str):
        raise ValueError("cost_tier must be a string or null")
    price_value = scenario.get("price_per_1000_text_records")
    price = None if price_value is None else float(cast(float, price_value))
    cost = azure_prompt_shield_guardrail_cost(usage_units, cost_tier, price)
    return None if cost is None else _decimal_str(cost)


def _breakdown_json(breakdown: CostBreakdown | None) -> Json:
    if breakdown is None:
        return None
    additional_costs = breakdown.get("additional_costs")
    result: dict[str, Json] = {field: breakdown.get(field) for field in _STRING_BREAKDOWN_FIELDS}
    for field in _DECIMAL_BREAKDOWN_FIELDS:
        value = breakdown.get(field)
        result[field] = None if value is None else _decimal_str(cast(float, value))
    result["additional_costs"] = (
        None
        if additional_costs is None
        else {key: _decimal_str(value) for key, value in additional_costs.items()}
    )
    extra: dict[str, Json] = {
        key: cast("Json", value)
        for key, value in breakdown.items()
        if key not in _KNOWN_BREAKDOWN_FIELDS
    }
    result["extra"] = extra
    return result


def _breakdown_steps(breakdown_input: dict[str, Json] | None) -> list[Json]:
    if breakdown_input is None:
        return []
    existing_value = breakdown_input.get("existing")
    current = None if existing_value is None else cast("CostBreakdown", existing_value)
    costs_value = breakdown_input.get("guardrail_costs", [])
    if not isinstance(costs_value, list):
        raise ValueError("guardrail_costs must be a JSON array")
    steps: list[Json] = []
    for cost_value in costs_value:
        current = cost_breakdown_with_guardrail(current, float(cast(float, cost_value)))
        steps.append(_breakdown_json(current))
    return steps


def run(input_value: dict[str, Json]) -> Json:
    guardrail_information = input_value.get("guardrail_information")
    scalar_cost = guardrail_information_cost(guardrail_information)
    rollup = [_rollup_entry(entry) for entry in _entries_of(guardrail_information)]

    azure_value = input_value.get("azure", [])
    if not isinstance(azure_value, list):
        raise ValueError("azure must be a JSON array")
    azure = [_azure_scenario_cost(cast("dict[str, Json]", scenario)) for scenario in azure_value]

    breakdown_value = input_value.get("breakdown")
    breakdown_steps = _breakdown_steps(
        None if breakdown_value is None else cast("dict[str, Json]", breakdown_value)
    )

    return {
        "scalar_cost": _decimal_str(scalar_cost),
        "rollup": rollup,
        "azure": azure,
        "breakdown_steps": breakdown_steps,
    }
