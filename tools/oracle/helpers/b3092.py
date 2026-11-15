from datetime import UTC, datetime
from typing import Literal, TypeAlias, TypedDict

from litellm.proxy.management_endpoints.common_daily_activity import (
    daily_activity_scope,
    spend_logs_window,
)
from pydantic import JsonValue


class WindowCall(TypedDict):
    function: Literal["spend_logs_window"]
    dates: list[str | None]


class ScopeCall(TypedDict):
    function: Literal["daily_activity_scope"]
    table: str
    entity_id_field: str
    entity_id: str | list[str] | None
    exclude_entity_ids: list[str] | None
    api_key: str | list[str] | None
    start_date: str
    end_date: str
    model: str | None
    timezone_offset_minutes: int | None
    include_current_utc_day: bool


Input: TypeAlias = WindowCall | ScopeCall


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "spend_logs_window":
        return window_value(input_value["dates"])
    return scope_value(input_value)


def window_value(dates: list[str | None]) -> dict[str, JsonValue]:
    window = spend_logs_window(frozenset(dates))
    if window is None:
        return {"window": None}
    return {"window": [utc_text(window[0]), utc_text(window[1])]}


def utc_text(moment: datetime) -> str:
    if moment.tzinfo is None:
        return moment.isoformat(timespec="microseconds")
    return moment.astimezone(UTC).replace(tzinfo=None).isoformat(
        timespec="microseconds"
    )


def scope_value(call: ScopeCall) -> dict[str, JsonValue]:
    scope = daily_activity_scope(
        table=call["table"],
        entity_id_field=call["entity_id_field"],
        entity_id=call["entity_id"],
        exclude_entity_ids=call["exclude_entity_ids"],
        api_key=call["api_key"],
        start_date=call["start_date"],
        end_date=call["end_date"],
        model=call["model"],
        timezone_offset_minutes=call["timezone_offset_minutes"],
        include_current_utc_day=call["include_current_utc_day"],
    )
    return {
        "scope": {
            "table": scope.table.value,
            "entity_id_field": scope.entity_id_field,
            "entity_ids": optional_texts(scope.entity_ids),
            "exclude_entity_ids": texts(scope.exclude_entity_ids),
            "api_keys": optional_texts(scope.api_keys),
            "start_date": scope.start_date,
            "end_date": scope.end_date,
            "model": scope.model,
            "timezone_offset_minutes": scope.timezone_offset_minutes,
            "include_current_utc_day": scope.include_current_utc_day,
        }
    }


def texts(values: tuple[str, ...]) -> list[JsonValue]:
    result: list[JsonValue] = []
    for value in values:
        result.append(value)
    return result


def optional_texts(values: tuple[str, ...] | None) -> list[JsonValue] | None:
    if values is None:
        return None
    return texts(values)
