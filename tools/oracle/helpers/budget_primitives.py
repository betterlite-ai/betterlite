import os
import time as time_module
from datetime import datetime, tzinfo
from typing import Any, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils import duration_parser
from litellm.models.organization import LiteLLM_OrganizationTable
from litellm.models.team import LiteLLM_TeamTable
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth.budget_throttle import (
    budget_throttle_percentage,
    should_throttle_budget_exceeded,
    throttled_limit,
)
from litellm.proxy.spend_tracking import budget_reservation
from litellm.proxy.spend_tracking.budget_reservation import (
    _coerce_window,  # pyright: ignore[reportPrivateUsage]
    get_budget_window_start,
)
from litellm.proxy.spend_tracking.carried_budget_state import (
    carried_budget_metadata,
    carry_organization_budget_state,
    carry_team_and_user_budget_state,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class FrozenDatetime(datetime):
    fixed: datetime | None = None

    @classmethod
    def now(cls, tz: tzinfo | None = None) -> datetime:
        fixed = cls.fixed
        if fixed is None:
            raise RuntimeError("no frozen time")
        return fixed if tz is None else fixed.astimezone(tz)


class DumpedModel:
    def __init__(self, value: Json) -> None:
        self._value = value

    def model_dump(self) -> Json:
        return self._value


class RaisingModel:
    def model_dump(self) -> Json:
        raise ValueError("model dump failed")


def object_fields(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def object_value(value: Json, name: str) -> dict[str, Any]:  # pyright: ignore[reportExplicitAny]
    return object_fields(value, name)


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def required(value: dict[str, Json], name: str) -> Json:
    if name not in value:
        raise ValueError(f"{name} is required")
    return value[name]


def injected(value: dict[str, Json]) -> float | None:
    return cast(float | None, required(value, "percentage"))


def token(value: Json) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(**object_value(value, "token"))  # pyright: ignore[reportAny]


def state(token_value: UserAPIKeyAuth) -> Json:
    return {
        "metadata": cast(Json, dict(carried_budget_metadata(token_value))),
        "team_model_max_budget": cast(Json, token_value.team_model_max_budget),
        "organization_alias": token_value.organization_alias,
    }


def throttle_percentage(value: dict[str, Json]) -> Json:
    litellm.budget_exceeded_throttle_percentage = injected(value)
    return {"percentage": budget_throttle_percentage()}


def should_throttle(value: dict[str, Json]) -> Json:
    litellm.budget_exceeded_throttle_percentage = injected(value)
    valid_token = token(required(value, "token"))
    return {"throttle": should_throttle_budget_exceeded(valid_token)}


def limit_call(value: dict[str, Json]) -> Json:
    limit = value.get("limit")
    configured = value.get("percentage")
    return {
        "limit": throttled_limit(
            cast(int | None, limit), cast(float | None, configured)
        )
    }


def window_input(value: Json) -> object:
    shape = object_fields(value, "window")
    if "value" in shape:
        return shape["value"]
    if "model" in shape:
        return DumpedModel(shape["model"])
    if "raising_model" in shape:
        return RaisingModel()
    raise ValueError("unknown window shape")


def coerce_window_call(value: dict[str, Json]) -> Json:
    window = _coerce_window(window_input(required(value, "window")))
    return {"window": cast(Json, dict(window))}


def window_start_call(value: dict[str, Json]) -> Json:
    window = window_input(required(value, "window"))
    now = datetime.fromisoformat(string_value(required(value, "now"), "now"))
    if now.tzinfo is None:
        raise ValueError("now must carry an offset")
    FrozenDatetime.fixed = now
    os.environ["TZ"] = "UTC"
    time_module.tzset()
    with (
        patch.object(budget_reservation, "datetime", FrozenDatetime),
        patch.object(duration_parser.time_module, "time", return_value=now.timestamp()),  # pyright: ignore[reportUnknownMemberType, reportPrivateImportUsage]
    ):
        start = get_budget_window_start(window)
    if start is None:
        return {"start": None, "offset_seconds": None}
    offset = start.utcoffset()
    return {
        "start": start.replace(tzinfo=None).isoformat(),
        "offset_seconds": None if offset is None else int(offset.total_seconds()),
    }


def carry_team_user(value: dict[str, Json]) -> Json:
    valid_token = token(required(value, "token"))
    raw_team = value.get("team")
    raw_user = value.get("user")
    team = (
        None
        if raw_team is None
        else LiteLLM_TeamTable(**object_value(raw_team, "team"))  # pyright: ignore[reportAny]
    )
    user = (
        None
        if raw_user is None
        else LiteLLM_UserTable(**object_value(raw_user, "user"))  # pyright: ignore[reportAny]
    )
    carry_team_and_user_budget_state(valid_token, team, user)
    return state(valid_token)


def carry_organization(value: dict[str, Json]) -> Json:
    valid_token = token(required(value, "token"))
    organization = LiteLLM_OrganizationTable(
        **object_value(required(value, "organization"), "organization")  # pyright: ignore[reportAny]
    )
    carry_organization_budget_state(valid_token, organization)
    return state(valid_token)


def carried_metadata_call(value: dict[str, Json]) -> Json:
    return state(token(required(value, "token")))


def dispatch(function: str, value: dict[str, Json]) -> Json:
    result: Json
    match function:
        case "throttle_percentage":
            result = throttle_percentage(value)
        case "should_throttle":
            result = should_throttle(value)
        case "throttled_limit":
            result = limit_call(value)
        case "coerce_window":
            result = coerce_window_call(value)
        case "window_start":
            result = window_start_call(value)
        case "carry_team_user":
            result = carry_team_user(value)
        case "carry_organization":
            result = carry_organization(value)
        case "carried_metadata":
            result = carried_metadata_call(value)
        case _:
            raise ValueError(f"unknown function {function}")
    return result


def run_call(call: Json) -> Json:
    entry = object_fields(call, "call")
    function = string_value(required(entry, "function"), "function")
    value = object_fields(required(entry, "value"), "value")
    try:
        result = dispatch(function, value)
    except Exception:  # noqa: BLE001 - every LiteLLM failure is the outcome the case compares
        return {"ok": False}
    return {"ok": True, "value": result}


def run(input_value: dict[str, Json]) -> Json:
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return cast(Json, [run_call(call) for call in calls])
