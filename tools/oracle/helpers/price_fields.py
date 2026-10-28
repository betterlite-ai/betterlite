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


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    source = input_value.get("source")
    if isinstance(source, str):
        input_value = cast(dict[str, Json], json.loads(source, parse_float=Decimal))
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
        if "cost" in name or "multiplier" in name:
            values[name] = _decimal(name, value)
    off_peak_value: Json = None
    if off_peak is not None:
        off_peak_object = _object("off_peak_pricing", off_peak)
        off_peak_value = {
            **{
                key: _decimal(f"off_peak_pricing.{key}", off_peak_object[key])
                if key in off_peak_object
                else None
                for key in [
                    "input_cost_per_token",
                    "output_cost_per_token",
                    "cache_read_input_token_cost",
                ]
            },
            "windows": off_peak_object.get("windows", []),
            **{
                key: _decimal(f"off_peak_pricing.{key}", off_peak_object[key])
                for key in ["cache_creation_input_token_cost", "output_cost_per_reasoning_token"]
                if key in off_peak_object
            },
            **(
                {"hours_utc": off_peak_object["hours_utc"]}
                if "hours_utc" in off_peak_object
                else {}
            ),
            **(
                {"weekday_timezone": off_peak_object["weekday_timezone"]}
                if "weekday_timezone" in off_peak_object
                else {}
            ),
        }
    tier_values: list[Json] = []
    for index, tier in enumerate(tiers):
        if not isinstance(tier, dict):
            raise PriceFieldError(f"price field tiered_pricing[{index}] must be a JSON object")
        tier_values.append(
            cast(
                Json,
                {
                    **{
                        key: _decimal(f"tiered_pricing[{index}].{key}", tier[key])
                        if key in tier
                        else None
                        for key in [
                            "input_cost_per_token",
                            "output_cost_per_token",
                            "output_cost_per_reasoning_token",
                        ]
                    },
                    "range": _range(tier, "range", f"tiered_pricing[{index}].range"),
                    "max_results_range": _range(
                        tier, "max_results_range", f"tiered_pricing[{index}].max_results_range"
                    ),
                    **{
                        key: _decimal(f"tiered_pricing[{index}].{key}", tier[key])
                        for key in [
                            "cache_read_input_token_cost",
                            "cache_creation_input_token_cost",
                            "input_cost_per_query",
                        ]
                        if key in tier
                    },
                },
            )
        )
    return cast(
        dict[str, Json],
        {
            **values,
            "provider_specific_entry": {
                key: _decimal(f"provider_specific_entry.{key}", provider[key])
                if key in provider
                else None
                for key in ["us", "fast"]
            },
            "guardrail_cost_per_unit": {
                key: _decimal(f"guardrail_cost_per_unit.{key}", value)
                for key, value in guardrails.items()
            },
            "metadata": {
                key: _decimal(f"metadata.{key}", metadata[key]) if key in metadata else None
                for key in ["original_pricing_per_hour", "original_pricing_per_minute"]
            },
            "sample_spec": {
                key: (_decimal(f"sample_spec.{key}", sample[key]) if key in sample else None)
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
                    key: (
                        _decimal(f"search_context_cost_per_query.{key}", search[key])
                        if key in search
                        else None
                    )
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
