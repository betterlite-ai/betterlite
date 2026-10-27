from decimal import Decimal
from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _decimal(value: Json) -> str:
    if value is None:
        raise ValueError("price field must not be null")
    if isinstance(value, bool) or not isinstance(value, (int, float, str)):
        raise ValueError("price field must be a JSON number")
    return format(Decimal(str(value)), "f")


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    values: dict[str, str] = {}
    for name, value in input_value.items():
        if name.startswith(("input_", "output_", "cache_")):
            values[name] = _decimal(value)
    provider = cast(dict[str, Json], input_value.get("provider_specific_entry", {}))
    guardrails = cast(dict[str, Json], input_value.get("guardrail_cost_per_unit", {}))
    return {
        "values": cast(Json, values),
        "provider_specific_us": _decimal(provider["us"]) if "us" in provider else None,
        "provider_specific_fast": _decimal(provider["fast"]) if "fast" in provider else None,
        "guardrail_units": cast(Json, {key: _decimal(value) for key, value in guardrails.items()}),
    }
