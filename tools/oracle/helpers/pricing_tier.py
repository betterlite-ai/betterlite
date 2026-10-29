from decimal import Decimal
from typing import TypeAlias

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_RANGE_LENGTH = 2


class TieredPricingError(ValueError):
    pass


def _coerce_cost_per_token(value: Json) -> float:
    if value is None:
        return 0.0
    if isinstance(value, str):
        try:
            return float(value)
        except ValueError:
            return 0.0
    if isinstance(value, (bool, int, float)):
        return float(value)
    raise TieredPricingError(
        f"a tier rate must be a number, string, boolean or null, not {type(value).__name__}"
    )


def _tier_range(tier: dict[str, Json], default: list[Json]) -> list[Json]:
    value = tier.get("range", default)
    if not isinstance(value, list):
        raise TieredPricingError("a tier's range must be a JSON array")
    return value


def _range_endpoint(value: Json) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TieredPricingError("a range endpoint must be a number")
    return float(value)


def select_tier_for_input(tiered_pricing: list[Json], input_tokens: int) -> Json:
    if not tiered_pricing or input_tokens <= 0:
        return None
    tiers: list[dict[str, Json]] = []
    for tier in tiered_pricing:
        if not isinstance(tier, dict):
            raise TieredPricingError("a tier must be a JSON object")
        tiers.append(tier)
    sorted_tiers = sorted(
        tiers, key=lambda tier: _range_endpoint(_tier_range(tier, [0, 0])[0])
    )
    valid_tiers = [tier for tier in sorted_tiers if len(_tier_range(tier, [])) == _RANGE_LENGTH]
    if not valid_tiers:
        return None
    matching = [
        tier
        for tier in valid_tiers
        if _range_endpoint(_tier_range(tier, [])[0])
        < input_tokens
        <= _range_endpoint(_tier_range(tier, [])[1])
    ]
    return matching[0] if matching else valid_tiers[-1]


def tier_rate(tier: dict[str, Json], cost_key: str, fallback_cost_key: str | None = None) -> float:
    primary = tier.get(cost_key)
    if primary is not None:
        return _coerce_cost_per_token(primary)
    fallback = tier.get(fallback_cost_key, 0) if fallback_cost_key is not None else 0
    return _coerce_cost_per_token(fallback)


def _normalized(rate: float) -> str:
    return format(Decimal(repr(rate)).normalize(), "f")


def run(input_value: dict[str, Json]) -> Json:
    tiers_value = input_value.get("tiers", [])
    if not isinstance(tiers_value, list):
        raise TieredPricingError("tiers must be a JSON array")
    tokens_value = input_value.get("tokens")
    if not isinstance(tokens_value, int) or isinstance(tokens_value, bool):
        raise TieredPricingError("tokens must be a JSON integer")
    cost_key_value = input_value.get("cost_key")
    if not isinstance(cost_key_value, str):
        raise TieredPricingError("cost_key must be a string")
    fallback_cost_key_value = input_value.get("fallback_cost_key")
    if fallback_cost_key_value is not None and not isinstance(fallback_cost_key_value, str):
        raise TieredPricingError("fallback_cost_key must be a string or null")
    selected = select_tier_for_input(tiers_value, tokens_value)
    if not isinstance(selected, dict):
        return {"selected_range": None, "rate": None}
    rate = tier_rate(selected, cost_key_value, fallback_cost_key_value)
    return {"selected_range": selected.get("range"), "rate": _normalized(rate)}
