from decimal import Decimal
from typing import TypedDict, cast

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    base_cost: str
    provider: str | None
    discount_config: dict[str, Json]
    margin_config: dict[str, Json]


def number(value: Json) -> Decimal | None:
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return Decimal(str(value))
    return None


def object_number(value: Json, key: str) -> Decimal:
    if not isinstance(value, dict):
        return Decimal(0)
    field = value.get(key)
    if isinstance(field, (int, float, str)) and not isinstance(field, bool):
        try:
            return Decimal(str(field))
        except (ValueError, TypeError):
            return Decimal(0)
    return Decimal(0)


def run(data: Input) -> Json:
    base = Decimal(data["base_cost"])
    discount_value = data["discount_config"].get(data["provider"] or "")
    discount_percent = number(discount_value) or Decimal(0)
    saved = (
        base * discount_percent
        if discount_value is not None and discount_percent
        else Decimal(0)
    )
    discount = {
        "final_cost": float(base - saved),
        "discount_percent": float(discount_percent),
        "saved_amount": float(saved),
    }
    margins = data["margin_config"]
    margin_value = (
        margins.get(data["provider"])
        if data["provider"] in margins
        else margins.get("global")
    )
    if isinstance(margin_value, (int, float)) and not isinstance(margin_value, bool):
        percent = number(margin_value) or Decimal(0)
        fixed = Decimal(0)
    else:
        percent = object_number(margin_value, "percentage")
        fixed = object_number(margin_value, "fixed_amount")
    total = base * percent + fixed if margin_value is not None else Decimal(0)
    margin = {
        "final_cost": float(base + total),
        "margin_percent": float(percent),
        "fixed_amount": float(fixed),
        "total_amount": float(total),
    }
    return cast(Json, {"discount": discount, "margin": margin})
