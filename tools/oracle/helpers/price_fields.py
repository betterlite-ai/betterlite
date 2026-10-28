from decimal import Decimal
from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class PriceFieldError(ValueError):
    pass


def _decimal(path: str, value: Json) -> str:
    if value is None:
        raise PriceFieldError(f"price field {path} must not be null")
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise PriceFieldError(f"price field {path} must be a JSON number")
    return format(Decimal(str(value)), "f")


def _object(path: str, value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise PriceFieldError(f"price field {path} must be a JSON object")
    return value


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    values: dict[str, str] = {}
    for name, value in input_value.items():
        nested = {
            "provider_specific_entry",
            "guardrail_cost_per_unit",
            "metadata",
            "sample_spec",
            "off_peak_pricing",
            "tiered_pricing",
            "search_context_cost_per_query",
        }
        if name in nested:
            continue
        priced = name.startswith(("input_", "output_", "cache_"))
        if priced or "_cost_" in name or "_multiplier" in name:
            values[name] = _decimal(name, value)
    provider = _object("provider_specific_entry", input_value.get("provider_specific_entry", {}))
    guardrails = _object("guardrail_cost_per_unit", input_value.get("guardrail_cost_per_unit", {}))
    metadata = _object("metadata", input_value.get("metadata", {}))
    sample = _object("sample_spec", input_value.get("sample_spec", {}))
    off_peak = _object("off_peak_pricing", input_value.get("off_peak_pricing", {}))
    search = _object(
        "search_context_cost_per_query", input_value.get("search_context_cost_per_query", {})
    )
    tiers = input_value.get("tiered_pricing", [])
    if not isinstance(tiers, list):
        raise PriceFieldError("price field tiered_pricing must be a JSON array")
    return cast(dict[str, Json], {
        "values": values,
        "provider_specific_us": _decimal("provider_specific_entry.us", provider["us"])
        if "us" in provider
        else None,
        "provider_specific_fast": _decimal("provider_specific_entry.fast", provider["fast"])
        if "fast" in provider
        else None,
        "guardrail_units": {
            key: _decimal(f"guardrail_cost_per_unit.{key}", value)
            for key, value in guardrails.items()
        },
        "metadata": {
            key: _decimal(f"metadata.{key}", metadata[key])
            for key in ("original_pricing_per_hour", "original_pricing_per_minute")
            if key in metadata
        },
        "sample_spec": {
            key: _decimal(f"sample_spec.{key}", sample[key]) for key in sample if "cost" in key
        },
        "off_peak": {
            key: _decimal(f"off_peak_pricing.{key}", off_peak[key])
            for key in off_peak
            if "cost" in key
        } if "off_peak_pricing" in input_value else None,
        "search_context": {
            key: _decimal(f"search_context_cost_per_query.{key}", value)
            for key, value in search.items()
        },
        "tiers": len(tiers),
    })
