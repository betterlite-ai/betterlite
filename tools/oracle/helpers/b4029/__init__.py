from datetime import date
from typing import Final, TypedDict

from litellm.integrations.SlackAlerting.user_spend_alerts import (
    UserSpendAlertEvent,
    UserSpendRow,
    evaluate_user_spend,
)
from litellm.types.integrations.slack_alerting import SlackAlertingArgs
from pydantic import TypeAdapter


class RowInput(TypedDict):
    user_id: str
    daily_spend: float
    monthly_spend: float
    baseline_spend: float


class SettingsInput(TypedDict):
    daily_threshold: float | None
    monthly_threshold: float | None
    spend_anomaly_multiplier: float
    spend_anomaly_baseline_days: int
    spend_anomaly_min_spend: float


class Input(TypedDict):
    row: RowInput
    settings: SettingsInput
    today: str
    thresholds_enabled: bool
    anomalies_enabled: bool


ARGS: Final = TypeAdapter(SlackAlertingArgs)


def settings_for(settings: SettingsInput) -> SlackAlertingArgs:
    values: dict[str, float | int | None] = {
        "daily_spend_per_user_threshold": settings["daily_threshold"],
        "monthly_spend_per_user_threshold": settings["monthly_threshold"],
        "spend_anomaly_multiplier": settings["spend_anomaly_multiplier"],
        "spend_anomaly_baseline_days": settings["spend_anomaly_baseline_days"],
        "spend_anomaly_min_spend": settings["spend_anomaly_min_spend"],
    }
    return ARGS.validate_python(values)


def event_value(event: UserSpendAlertEvent) -> dict[str, str | int]:
    return {
        "kind": event.kind,
        "alert_type": event.alert_type.value,
        "message": event.message,
        "cache_key": event.cache_key,
        "cache_ttl": event.cache_ttl,
    }


def run(input_value: Input) -> list[dict[str, str | int]]:
    raw_row = input_value["row"]
    row = UserSpendRow(
        user_id=raw_row["user_id"],
        daily_spend=raw_row["daily_spend"],
        monthly_spend=raw_row["monthly_spend"],
        baseline_spend=raw_row["baseline_spend"],
    )
    events = evaluate_user_spend(
        row=row,
        args=settings_for(input_value["settings"]),
        today=date.fromisoformat(input_value["today"]),
        thresholds_enabled=input_value["thresholds_enabled"],
        anomalies_enabled=input_value["anomalies_enabled"],
    )
    return [event_value(event) for event in events]
