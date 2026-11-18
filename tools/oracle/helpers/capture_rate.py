from datetime import date
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy.spend_tracking.spend_capture_rate import (
    ProviderBillingCredentialMissing,
    ProviderBillingRequestFailed,
    alert_message,
    compute_capture_rate,
)
from litellm.types.proxy.spend_capture_rate import CaptureRateReport
from pydantic import JsonValue, TypeAdapter


class ReportFields(TypedDict):
    provider: Literal["openai"]
    start_date: str
    end_date: str
    threshold: float
    captured_by_day: dict[str, float]
    billed_by_day: dict[str, float]


class ReportCase(ReportFields):
    function: Literal["report"]


class AlertReportCase(ReportFields):
    function: Literal["alert_report"]


class CredentialMissingCase(TypedDict):
    function: Literal["alert_credential_missing"]
    provider: Literal["openai"]
    env_var: str


class RequestFailedCase(TypedDict):
    function: Literal["alert_request_failed"]
    provider: Literal["openai"]
    detail: str


Call: TypeAlias = (
    ReportCase | AlertReportCase | CredentialMissingCase | RequestFailedCase
)


class Input(TypedDict):
    cases: dict[str, Call]


class DayRecord(TypedDict):
    date: str
    captured_spend: float
    provider_spend: float
    capture_rate: float | None


class ReportRecord(TypedDict):
    provider: Literal["openai"]
    start_date: str
    end_date: str
    captured_spend: float
    provider_spend: float
    capture_rate: float | None
    threshold: float
    below_threshold: bool
    days: list[DayRecord]


REPORT: Final = TypeAdapter(ReportRecord)


def computed(case: ReportFields) -> CaptureRateReport:
    return compute_capture_rate(
        provider=case["provider"],
        start_date=date.fromisoformat(case["start_date"]),
        end_date=date.fromisoformat(case["end_date"]),
        captured_by_day=case["captured_by_day"],
        billed_by_day=case["billed_by_day"],
        threshold=case["threshold"],
    )


def report(case: ReportFields) -> ReportRecord:
    return REPORT.validate_json(computed(case).model_dump_json())


def report_json(record: ReportRecord) -> dict[str, JsonValue]:
    days: list[JsonValue] = []
    for day in record["days"]:
        entry: dict[str, JsonValue] = {
            "date": day["date"],
            "captured_spend": day["captured_spend"],
            "provider_spend": day["provider_spend"],
            "capture_rate": day["capture_rate"],
        }
        days.append(entry)
    fields: dict[str, JsonValue] = {
        "provider": record["provider"],
        "start_date": record["start_date"],
        "end_date": record["end_date"],
        "captured_spend": record["captured_spend"],
        "provider_spend": record["provider_spend"],
        "capture_rate": record["capture_rate"],
        "threshold": record["threshold"],
        "below_threshold": record["below_threshold"],
        "days": days,
    }
    return fields


def answer(case: Call) -> JsonValue:
    if case["function"] == "report":
        return {"report": report_json(report(case))}
    if case["function"] == "alert_report":
        return {"alert": alert_message(computed(case))}
    if case["function"] == "alert_credential_missing":
        missing = ProviderBillingCredentialMissing(
            provider=case["provider"], env_var=case["env_var"]
        )
        return {"alert": alert_message(missing)}
    failed = ProviderBillingRequestFailed(
        provider=case["provider"], detail=case["detail"]
    )
    return {"alert": alert_message(failed)}


def run(input_value: Input) -> JsonValue:
    resolved: dict[str, JsonValue] = {}
    for name, case in input_value["cases"].items():
        resolved[name] = answer(case)
    return resolved
