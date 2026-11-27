import asyncio
from datetime import UTC, datetime
from typing import Final, TypedDict

import litellm
from litellm.models.budget import LiteLLM_BudgetTableFull
from pydantic import JsonValue, TypeAdapter

from .inputs import (
    CustomerCall,
    DefaultBudgetCall,
    Input,
    KeyCall,
    OrgCall,
    TeamCall,
    UserCall,
)
from .logger import BudgetLogger, SampleRow
from .lookups import FakePrismaClient, ScriptedLookups, patched

DEFAULT_NOW: Final = datetime(2026, 1, 1, tzinfo=UTC)
_JSON: Final = TypeAdapter(JsonValue)


class LookupOutput(TypedDict):
    samples: list[SampleRow]
    lookups: list[dict[str, JsonValue]]


class BudgetText(TypedDict):
    max_budget: str | None
    budget_reset_at: str | None


class DefaultOutput(TypedDict):
    budget: BudgetText | None
    lookups: list[dict[str, JsonValue]]


def lookups_output(logger: BudgetLogger, scripted: ScriptedLookups) -> JsonValue:
    payload: LookupOutput = {"samples": logger.samples(), "lookups": scripted.entries}
    return _JSON.validate_python(payload)


def budget_text(row: LiteLLM_BudgetTableFull | None) -> BudgetText | None:
    if row is None:
        return None
    max_budget = None if row.max_budget is None else format(row.max_budget, ".6f")
    reset_at = None if row.budget_reset_at is None else str(int(row.budget_reset_at.timestamp()))
    return {"max_budget": max_budget, "budget_reset_at": reset_at}


def default_output(row: LiteLLM_BudgetTableFull | None, scripted: ScriptedLookups) -> JsonValue:
    payload: DefaultOutput = {"budget": budget_text(row), "lookups": scripted.entries}
    return _JSON.validate_python(payload)


def team(call: TeamCall) -> JsonValue:
    scripted = ScriptedLookups(call["lookup"], database=True)
    logger = BudgetLogger(
        call["labels_by_metric"], datetime.fromisoformat(call["now"]), noop=call["noop"]
    )
    with patched(scripted):
        asyncio.run(logger.team(call))
    return lookups_output(logger, scripted)


def org(call: OrgCall) -> JsonValue:
    scripted = ScriptedLookups(call["lookup"], database=call["database"])
    logger = BudgetLogger(
        call["labels_by_metric"], datetime.fromisoformat(call["now"]), noop=call["noop"]
    )
    with patched(scripted):
        asyncio.run(logger.org(call))
    return lookups_output(logger, scripted)


def customer(call: CustomerCall) -> JsonValue:
    litellm.enable_end_user_cost_tracking_prometheus_only = call["enable_prometheus_only"]
    litellm.disable_end_user_cost_tracking = call["disable_end_user_cost_tracking"]
    scripted = ScriptedLookups(call["lookup"], database=True)
    logger = BudgetLogger(
        call["labels_by_metric"], datetime.fromisoformat(call["now"]), noop=call["noop"]
    )
    with patched(scripted):
        asyncio.run(logger.customer(call))
    return lookups_output(logger, scripted)


def key(call: KeyCall) -> JsonValue:
    scripted = ScriptedLookups(call["lookup"], database=True)
    logger = BudgetLogger(
        call["labels_by_metric"], datetime.fromisoformat(call["now"]), noop=call["noop"]
    )
    with patched(scripted):
        asyncio.run(logger.key(call))
    return lookups_output(logger, scripted)


def user(call: UserCall) -> JsonValue:
    scripted = ScriptedLookups(call["lookup"], database=True)
    logger = BudgetLogger(
        call["labels_by_metric"], datetime.fromisoformat(call["now"]), noop=call["noop"]
    )
    with patched(scripted):
        asyncio.run(logger.user(call))
    return lookups_output(logger, scripted)


def default_budget(call: DefaultBudgetCall) -> JsonValue:
    litellm.max_end_user_budget_id = call["default_budget_id"]
    scripted = ScriptedLookups(call["lookup"], database=True)
    logger = BudgetLogger({}, DEFAULT_NOW, noop=True)
    row = asyncio.run(logger.default_customer_budget(FakePrismaClient(scripted)))
    return default_output(row, scripted)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "team":
        return team(input_value)
    if input_value["function"] == "org":
        return org(input_value)
    if input_value["function"] == "customer":
        return customer(input_value)
    if input_value["function"] == "key":
        return key(input_value)
    if input_value["function"] == "user":
        return user(input_value)
    return default_budget(input_value)
