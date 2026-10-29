import json
from decimal import Decimal, Inexact, localcontext
from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | Decimal | str | list["Json"] | dict[str, "Json"]


class PriceFieldError(ValueError):
    pass


def _decimal(path: str, value: Json) -> str:
    if value is None:
        raise PriceFieldError(f"price field {path} must not be null")
    if isinstance(value, bool) or not isinstance(value, (int, Decimal)):
        raise PriceFieldError(f"price field {path} must be a JSON number")
    with localcontext() as context:
        context.prec = 29
        context.traps[Inexact] = True
        try:
            return format(Decimal(value).normalize(), "f")
        except Inexact as error:
            raise PriceFieldError(f"price field {path} exceeds decimal precision") from error


def _object(path: str, value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise PriceFieldError(f"price field {path} must be a JSON object")
    return value


def _range(source: dict[str, Json], key: str, path: str) -> list[str] | None:
    value = source.get(key)
    if value is None:
        return None
    if not isinstance(value, list):
        raise PriceFieldError(f"price field {path} must be a JSON array")
    return [_decimal(f"{path}[{index}]", bound) for index, bound in enumerate(value)]


def _fixed_keys(prefix: str, source: dict[str, Json], keys: list[str]) -> dict[str, Json]:
    return {
        key: _decimal(f"{prefix}.{key}", source[key]) if key in source else None for key in keys
    }


_GROUP_KEYS = {
    "provider_specific_entry": ["us", "fast"],
    "metadata": ["original_pricing_per_hour", "original_pricing_per_minute"],
    "search_context_cost_per_query": [
        "search_context_size_low",
        "search_context_size_medium",
        "search_context_size_high",
    ],
}

_EXCLUDED_KEYS = {
    "provider_specific_entry",
    "guardrail_cost_per_unit",
    "metadata",
    "off_peak_pricing",
    "search_context_cost_per_query",
    "tiered_pricing",
}


def _flat_values(input_value: dict[str, Json]) -> dict[str, Json]:
    values: dict[str, Json] = {}
    for name, value in input_value.items():
        if name in _EXCLUDED_KEYS:
            continue
        if "cost" in name or "multiplier" in name:
            values[name] = _decimal(name, value)
    return values


def _guardrail_cost_per_unit(input_value: dict[str, Json]) -> dict[str, Json]:
    guardrails = _object("guardrail_cost_per_unit", input_value.get("guardrail_cost_per_unit", {}))
    return {
        key: _decimal(f"guardrail_cost_per_unit.{key}", value) for key, value in guardrails.items()
    }


def _search_context_cost_per_query(input_value: dict[str, Json]) -> Json:
    if "search_context_cost_per_query" not in input_value:
        return None
    search = _object("search_context_cost_per_query", input_value["search_context_cost_per_query"])
    return _fixed_keys(
        "search_context_cost_per_query", search, _GROUP_KEYS["search_context_cost_per_query"]
    )


def _off_peak_pricing(input_value: dict[str, Json]) -> Json:
    off_peak = input_value.get("off_peak_pricing")
    if off_peak is None:
        return None
    off_peak_object = _object("off_peak_pricing", off_peak)
    windowed = _fixed_keys(
        "off_peak_pricing",
        off_peak_object,
        ["input_cost_per_token", "output_cost_per_token", "cache_read_input_token_cost"],
    )
    tracked = {
        key: _decimal(f"off_peak_pricing.{key}", off_peak_object[key])
        for key in ["cache_creation_input_token_cost", "output_cost_per_reasoning_token"]
        if key in off_peak_object
    }
    passthrough = {
        key: off_peak_object[key]
        for key in ["hours_utc", "weekday_timezone"]
        if key in off_peak_object
    }
    return {
        **windowed,
        "windows": off_peak_object.get("windows", []),
        **tracked,
        **passthrough,
    }


def _tier(index: int, tier: Json) -> Json:
    if not isinstance(tier, dict):
        raise PriceFieldError(f"price field tiered_pricing[{index}] must be a JSON object")
    prefix = f"tiered_pricing[{index}]"
    windowed = _fixed_keys(
        prefix,
        tier,
        ["input_cost_per_token", "output_cost_per_token", "output_cost_per_reasoning_token"],
    )
    tracked = {
        key: _decimal(f"{prefix}.{key}", tier[key])
        for key in [
            "cache_read_input_token_cost",
            "cache_creation_input_token_cost",
            "input_cost_per_query",
        ]
        if key in tier
    }
    return cast(
        Json,
        {
            **windowed,
            "range": _range(tier, "range", f"{prefix}.range"),
            "max_results_range": _range(tier, "max_results_range", f"{prefix}.max_results_range"),
            **tracked,
        },
    )


def _tiered_pricing(input_value: dict[str, Json]) -> list[Json]:
    tiers = input_value.get("tiered_pricing", [])
    if not isinstance(tiers, list):
        raise PriceFieldError("price field tiered_pricing must be a JSON array")
    return [_tier(index, tier) for index, tier in enumerate(tiers)]


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    source = input_value.get("source")
    if isinstance(source, str):
        input_value = cast(dict[str, Json], json.loads(source, parse_float=Decimal))
    provider = _object("provider_specific_entry", input_value.get("provider_specific_entry", {}))
    metadata = _object("metadata", input_value.get("metadata", {}))
    return cast(
        dict[str, Json],
        {
            **_flat_values(input_value),
            "provider_specific_entry": _fixed_keys(
                "provider_specific_entry", provider, _GROUP_KEYS["provider_specific_entry"]
            ),
            "guardrail_cost_per_unit": _guardrail_cost_per_unit(input_value),
            "metadata": _fixed_keys("metadata", metadata, _GROUP_KEYS["metadata"]),
            "off_peak_pricing": _off_peak_pricing(input_value),
            "search_context_cost_per_query": _search_context_cost_per_query(input_value),
            "tiered_pricing": _tiered_pricing(input_value),
        },
    )
