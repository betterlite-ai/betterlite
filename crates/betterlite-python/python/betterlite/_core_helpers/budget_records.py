from decimal import Decimal
from typing import Final

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict


class BudgetRecord(TypedDict, total=False):
    total_budget: Decimal
    duration: int
    created_at: float
    last_updated_at: float
    current_cost: Decimal | int
    model_cost: dict[str, Decimal]


USERS: Final[TypeAdapter[dict[str, BudgetRecord]]] = TypeAdapter(dict[str, BudgetRecord])


def cost_decimal(value: Decimal | int | None) -> Decimal:
    if value is None:
        return Decimal(0)
    match value:
        case Decimal() as amount:
            return amount
        case int() as whole:
            return Decimal(whole)


def public_money(value: Decimal | int) -> JsonValue:
    match value:
        case Decimal() as amount:
            return float(amount)
        case int() as whole:
            return whole


def record_json(record: BudgetRecord) -> dict[str, JsonValue]:
    stored: dict[str, JsonValue] = {}
    total_budget = record.get("total_budget")
    if total_budget is not None:
        stored["total_budget"] = float(total_budget)
    duration = record.get("duration")
    if duration is not None:
        stored["duration"] = duration
    created_at = record.get("created_at")
    if created_at is not None:
        stored["created_at"] = created_at
    last_updated_at = record.get("last_updated_at")
    if last_updated_at is not None:
        stored["last_updated_at"] = last_updated_at
    current_cost = record.get("current_cost")
    if current_cost is not None:
        stored["current_cost"] = public_money(current_cost)
    model_cost = record.get("model_cost")
    if model_cost is not None:
        stored["model_cost"] = {name: float(cost) for name, cost in model_cost.items()}
    return stored
