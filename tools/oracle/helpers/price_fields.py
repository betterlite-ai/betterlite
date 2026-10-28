from decimal import Decimal
from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class PriceFieldError(ValueError):
    pass


def _decimal(path: str, value: Json) -> str:
    if value is None:
        raise PriceFieldError(f"price field {path} must not be null")
    if isinstance(value, bool) or not isinstance(value, (int, float, Decimal)):
        raise PriceFieldError(f"price field {path} must be a JSON number")
    return format(Decimal(str(value)).normalize(), "f")


def _object(path: str, value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise PriceFieldError(f"price field {path} must be a JSON object")
    return value


def _optional(source: dict[str, Json], key: str, path: str) -> str | None:
    return _decimal(path, source[key]) if key in source else None


def _range(source: dict[str, Json], key: str, path: str) -> list[str] | None:
    value = source.get(key)
    if value is None:
        return None
    if not isinstance(value, list):
        raise PriceFieldError(f"price field {path} must be a JSON array")
    return [_decimal(f"{path}[{index}]", bound) for index, bound in enumerate(value)]


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    provider = _object("provider_specific_entry", input_value.get("provider_specific_entry", {}))
    guardrails = _object("guardrail_cost_per_unit", input_value.get("guardrail_cost_per_unit", {}))
    metadata = _object("metadata", input_value.get("metadata", {}))
    sample = _object("sample_spec", input_value.get("sample_spec", {}))
    off_peak = input_value.get("off_peak_pricing")
    search = _object(
        "search_context_cost_per_query", input_value.get("search_context_cost_per_query", {})
    )
    tiers = input_value.get("tiered_pricing", [])
    if not isinstance(tiers, list):
        raise PriceFieldError("price field tiered_pricing must be a JSON array")
    values: dict[str, Json] = {}
    for name, value in input_value.items():
        if name in {
            "provider_specific_entry",
            "guardrail_cost_per_unit",
            "metadata",
            "sample_spec",
            "off_peak_pricing",
            "search_context_cost_per_query",
            "tiered_pricing",
        }:
            continue
        if (
            name.startswith(("input_", "output_", "cache_"))
            or "_cost_" in name
            or "_multiplier" in name
        ):
            values[name] = _decimal(name, value)
    off_peak_value: Json = None
    if off_peak is not None:
        off_peak_object = _object("off_peak_pricing", off_peak)
        off_peak_value = {
            "input_cost_per_token": _optional(
                off_peak_object, "input_cost_per_token", "off_peak_pricing.input_cost_per_token"
            ),
            "output_cost_per_token": _optional(
                off_peak_object, "output_cost_per_token", "off_peak_pricing.output_cost_per_token"
            ),
            "cache_read_input_token_cost": _optional(
                off_peak_object,
                "cache_read_input_token_cost",
                "off_peak_pricing.cache_read_input_token_cost",
            ),
            "windows": off_peak_object.get("windows"),
        }
    tier_values: list[Json] = []
    for index, tier in enumerate(tiers):
        if not isinstance(tier, dict):
            raise PriceFieldError(f"price field tiered_pricing[{index}] must be a JSON object")
        tier_values.append(
            {
                "input_cost_per_token": _optional(
                    tier, "input_cost_per_token", f"tiered_pricing[{index}].input_cost_per_token"
                ),
                "output_cost_per_token": _optional(
                    tier, "output_cost_per_token", f"tiered_pricing[{index}].output_cost_per_token"
                ),
                "output_cost_per_reasoning_token": _optional(
                    tier,
                    "output_cost_per_reasoning_token",
                    f"tiered_pricing[{index}].output_cost_per_reasoning_token",
                ),
                "range": _range(tier, "range", f"tiered_pricing[{index}].range"),
                "max_results_range": _range(
                    tier, "max_results_range", f"tiered_pricing[{index}].max_results_range"
                ),
            }
        )
    return cast(
        dict[str, Json],
        {
            **values,
            "provider_specific_entry": {
                "us": _optional(provider, "us", "provider_specific_entry.us"),
                "fast": _optional(provider, "fast", "provider_specific_entry.fast"),
            },
            "guardrail_cost_per_unit": {
                key: _decimal(f"guardrail_cost_per_unit.{key}", value)
                for key, value in guardrails.items()
            },
            "metadata": {
                "original_pricing_per_hour": _optional(
                    metadata, "original_pricing_per_hour", "metadata.original_pricing_per_hour"
                ),
                "original_pricing_per_minute": _optional(
                    metadata, "original_pricing_per_minute", "metadata.original_pricing_per_minute"
                ),
            },
            "sample_spec": {
                key: _optional(sample, key, f"sample_spec.{key}")
                for key in [
                    "computer_use_input_cost_per_1k_tokens",
                    "computer_use_output_cost_per_1k_tokens",
                    "file_search_cost_per_1k_calls",
                    "file_search_cost_per_gb_per_day",
                    "vector_store_cost_per_gb_per_day",
                ]
            },
            "off_peak_pricing": off_peak_value,
            "search_context_cost_per_query": (
                {
                    key: _optional(search, key, f"search_context_cost_per_query.{key}")
                    for key in [
                        "search_context_size_low",
                        "search_context_size_medium",
                        "search_context_size_high",
                    ]
                }
                if "search_context_cost_per_query" in input_value
                else None
            ),
            "tiered_pricing": tier_values,
        },
    )
