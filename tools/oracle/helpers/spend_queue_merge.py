from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy._types import (
    BaseDailySpendTransaction,
    DBSpendUpdateTransactions,
    Litellm_EntityType,
    SpendUpdateQueueItem,
)
from litellm.proxy.db.db_transaction_queue.daily_spend_update_queue import DailySpendUpdateQueue
from litellm.proxy.db.db_transaction_queue.spend_update_queue import SpendUpdateQueue

EntityKind: TypeAlias = Literal[
    "key",
    "user",
    "end_user",
    "team",
    "team_member",
    "organization",
    "organization_member",
    "project",
    "tag",
    "agent",
    "model_access_group",
    "proxy",
]


class EntityUpdate(TypedDict):
    entity_type: NotRequired[EntityKind]
    entity_id: NotRequired[str]
    response_cost: NotRequired[float | None]


class EntityStep(TypedDict):
    kind: Literal["entity_transactions"]
    updates: list[EntityUpdate]


class DailyStep(TypedDict):
    kind: Literal["daily"]
    updates: list[dict[str, BaseDailySpendTransaction]]


Step: TypeAlias = EntityStep | DailyStep


class Invalid(TypedDict):
    invalid: bool


class Input(TypedDict):
    steps: dict[str, Step]


StepResult: TypeAlias = DBSpendUpdateTransactions | dict[str, BaseDailySpendTransaction] | Invalid


def queue_item(update: EntityUpdate) -> SpendUpdateQueueItem:
    item: SpendUpdateQueueItem = {}
    kind = update.get("entity_type")
    if kind is not None:
        item["entity_type"] = Litellm_EntityType(kind)
    entity_id = update.get("entity_id")
    if entity_id is not None:
        item["entity_id"] = entity_id
    cost = update.get("response_cost")
    if cost is not None:
        item["response_cost"] = cost
    return item


def entity_transactions(updates: list[EntityUpdate]) -> StepResult:
    queue = SpendUpdateQueue.__new__(SpendUpdateQueue)
    items = [queue_item(update) for update in updates]
    return queue.get_aggregated_db_spend_update_transactions(items)


def daily_transactions(updates: list[dict[str, BaseDailySpendTransaction]]) -> StepResult:
    try:
        aggregated = DailySpendUpdateQueue.get_aggregated_daily_spend_update_transactions(updates)
    except (KeyError, TypeError):
        invalid: Invalid = {"invalid": True}
        return invalid
    return aggregated


def step_value(step: Step) -> StepResult:
    if step["kind"] == "entity_transactions":
        return entity_transactions(step["updates"])
    return daily_transactions(step["updates"])


def run(input_value: Input) -> dict[str, StepResult]:
    steps = input_value["steps"]
    return {name: step_value(step) for name, step in steps.items()}
