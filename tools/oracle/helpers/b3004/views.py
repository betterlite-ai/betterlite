from collections.abc import Mapping

from litellm.exceptions import BudgetExceededError
from litellm.proxy._types import LiteLLM_UserTable, UserAPIKeyAuth
from pydantic import JsonValue
from starlette.exceptions import HTTPException

from .fakes import ScriptedMemory
from .models import RESERVATION, Entry, TokenInput, UserInput


def token_object(data: TokenInput) -> UserAPIKeyAuth:
    limits: list[dict[str, JsonValue]] | None = None
    budget_limits = data.get("budget_limits")
    if budget_limits is not None:
        limits = [
            {"budget_duration": limit["budget_duration"], "max_budget": limit["max_budget"]}
            for limit in budget_limits
        ]
    metadata: dict[str, JsonValue] = {"throttle_on_budget_exceeded": data["throttle"]}
    return UserAPIKeyAuth(
        token=data["token"],
        max_budget=data["max_budget"],
        spend=data["spend"],
        tpm_limit=data["tpm_limit"],
        rpm_limit=data["rpm_limit"],
        user_id=data["user_id"],
        budget_limits=limits,
        metadata=metadata,
    )


def user_object(data: UserInput | None) -> LiteLLM_UserTable | None:
    if data is None:
        return None
    return LiteLLM_UserTable(
        user_id=data["user_id"], max_budget=data["max_budget"], spend=data["spend"]
    )


def entry_json(entry: Entry) -> JsonValue:
    described: dict[str, JsonValue] = {
        "counter_key": entry["counter_key"],
        "reserved_cost": entry["reserved_cost"],
        "applied_adjustment": entry["applied_adjustment"],
    }
    return described


def reservation_json(reservation: Mapping[str, JsonValue] | None) -> JsonValue:
    if reservation is None:
        return None
    parsed = RESERVATION.validate_python(reservation)
    entries: list[JsonValue] = [entry_json(entry) for entry in parsed["entries"]]
    described: dict[str, JsonValue] = {
        "reserved_cost": parsed["reserved_cost"],
        "entries": entries,
    }
    return described


def error_json(error: BudgetExceededError | HTTPException) -> JsonValue:
    described: dict[str, JsonValue] = {
        "class": type(error).__name__,
        "status": error.status_code,
    }
    return described


def memory_json(memory: ScriptedMemory) -> JsonValue:
    amounts: Mapping[str, JsonValue] = memory.values
    return dict(amounts)


def invalidated_json(memory: ScriptedMemory) -> JsonValue:
    listed: list[JsonValue] = []
    listed.extend(memory.deleted)
    return listed
