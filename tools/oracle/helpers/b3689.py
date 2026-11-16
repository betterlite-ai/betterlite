from collections.abc import Callable, Iterable, Mapping, Sequence
from datetime import datetime, tzinfo
from typing import Final, Literal, TypeAlias, TypedDict, TypeVar, cast

from litellm.proxy.guardrails import usage_endpoints as usage
from litellm.proxy.guardrails.usage_endpoints import UsageChartPoint, UsageOverviewRow
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException as StarletteHTTPException

JsonObject: TypeAlias = dict[str, JsonValue]
Case = TypeVar("Case")


class WindowCase(BaseModel):
    start_date: str | None
    end_date: str | None
    now: str


class ThresholdCase(BaseModel):
    fail_rate: float
    previous: float


class MetricRow(BaseModel):
    date: str
    id: str
    requests_evaluated: int | None
    passed_count: int | None
    blocked_count: int | None
    flagged_count: int | None

    def metric_id(self) -> str:
        return self.id


class DailyCase(BaseModel):
    reduce: Literal["aggregate", "previous", "chart"]
    rows: list[MetricRow]


class UnitsRow(BaseModel):
    usage_unit: str
    guardrail_id: str
    team_id: str
    api_key: str
    units: int
    untracked_units: int
    cost: float | None

    def id_of_guardrail(self) -> str:
        return self.guardrail_id


class SumCase(BaseModel):
    reduce: Literal["units", "untracked", "cost"]
    rows: list[UnitsRow]


class GroupCase(BaseModel):
    key: Literal["usage_unit", "team_id", "api_key", "guardrail_id"]
    reduce: Literal["units", "untracked", "cost"]
    rows: list[UnitsRow]


class RowsCase(BaseModel):
    rows: list[UnitsRow]


class GuardrailRecord(BaseModel):
    guardrail_id: str | None = None
    guardrail_name: str | None = None
    litellm_params: JsonObject | None = None
    guardrail_info: JsonObject | None = None


class PolicyRow(BaseModel):
    policy_id: str
    policy_name: str | None


class MetricTotals(TypedDict):
    requests: int
    passed: int
    blocked: int
    flagged: int


class Aggregates(BaseModel):
    metrics: dict[str, MetricTotals]
    previous_rates: dict[str, float]
    units: dict[str, dict[str, int]]
    costs: dict[str, float | None]
    untracked: dict[str, dict[str, int]]


class OverviewCase(BaseModel):
    records: list[JsonObject]
    aggregates: Aggregates


class Input(TypedDict):
    function: str
    cases: list[JsonObject]


class FixedClock:
    def __init__(self, moment: datetime) -> None:
        self.moment = moment

    def now(self, tz: tzinfo | None = None) -> datetime:
        if tz is None:
            return self.moment
        return self.moment.astimezone(tz)


WINDOW = cast("Callable[[str | None, str | None], tuple[str, str]]", usage._resolve_usage_window)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L53-L82
STATUS = cast("Callable[[float], str]", usage._status_from_fail_rate)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L345-L351
TREND = cast("Callable[[float, float], str]", usage._trend_from_comparison)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L354-L362
AGGREGATE = cast("Callable[[Sequence[MetricRow], Callable[[MetricRow], str]], Mapping[str, MetricTotals]]", usage._aggregate_daily_metrics)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L365-L378
PREVIOUS = cast("Callable[[Sequence[MetricRow], Callable[[MetricRow], str]], Mapping[str, float]]", usage._prev_fail_rates)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L381-L391
CHART = cast("Callable[[Sequence[MetricRow]], list[UsageChartPoint]]", usage._chart_from_metrics)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L394-L401
SUM_UNITS = cast("Callable[[Iterable[UnitsRow]], Mapping[str, int]]", usage._sum_counter_units)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L181-L186
SUM_UNTRACKED = cast("Callable[[Iterable[UnitsRow]], Mapping[str, int]]", usage._sum_untracked_units)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L189-L196
SUM_COST = cast("Callable[[Iterable[UnitsRow]], float | None]", usage._sum_tracked_cost)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L199-L203
ROW_UNTRACKED = cast("Callable[[UnitsRow], int]", usage._row_untracked_units)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L171-L174
ROW_COST = cast("Callable[[UnitsRow], float | None]", usage._row_tracked_cost)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L177-L179
BY_UNITS = cast("Callable[[Sequence[UnitsRow], Callable[[UnitsRow], str], Callable[[Iterable[UnitsRow]], Mapping[str, int]]], Mapping[str, Mapping[str, int]]]", usage._by)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L206-L212
BY_UNTRACKED = cast("Callable[[Sequence[UnitsRow], Callable[[UnitsRow], str], Callable[[Iterable[UnitsRow]], Mapping[str, int]]], Mapping[str, Mapping[str, int]]]", usage._by)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L206-L212
BY_COST = cast("Callable[[Sequence[UnitsRow], Callable[[UnitsRow], str], Callable[[Iterable[UnitsRow]], float | None]], Mapping[str, float | None]]", usage._by)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L206-L212
COUNTER = cast("Callable[[UnitsRow], str]", usage._counter_name)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L158-L160
TEAM = cast("Callable[[UnitsRow], str]", usage._team_of)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L162-L164
KEY = cast("Callable[[UnitsRow], str]", usage._key_of)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L166-L168
GUARDRAIL_ROWS = cast("Callable[[Sequence[GuardrailRecord], Mapping[str, MetricTotals], Mapping[str, float], Mapping[str, Mapping[str, int]], Mapping[str, float | None], Mapping[str, Mapping[str, int]]], list[UsageOverviewRow]]", usage._guardrail_overview_rows)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L436-L516
POLICY_ROWS = cast("Callable[[Sequence[PolicyRow], Mapping[str, MetricTotals], Mapping[str, float]], list[UsageOverviewRow]]", usage._policy_overview_rows)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L519-L550
METRIC_ID: Final[Callable[[MetricRow], str]] = MetricRow.metric_id  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L600
GUARDRAIL_ID: Final[Callable[[UnitsRow], str]] = UnitsRow.id_of_guardrail  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L602

KEYS: Final[Mapping[str, Callable[[UnitsRow], str]]] = {
    "usage_unit": COUNTER,
    "team_id": TEAM,
    "api_key": KEY,
    "guardrail_id": GUARDRAIL_ID,
}

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ROWS: Final[TypeAdapter[list[UsageOverviewRow]]] = TypeAdapter(list[UsageOverviewRow])
WINDOW_CASES: Final[TypeAdapter[list[WindowCase]]] = TypeAdapter(list[WindowCase])
THRESHOLD_CASES: Final[TypeAdapter[list[ThresholdCase]]] = TypeAdapter(list[ThresholdCase])
DAILY_CASES: Final[TypeAdapter[list[DailyCase]]] = TypeAdapter(list[DailyCase])
SUM_CASES: Final[TypeAdapter[list[SumCase]]] = TypeAdapter(list[SumCase])
GROUP_CASES: Final[TypeAdapter[list[GroupCase]]] = TypeAdapter(list[GroupCase])
ROWS_CASES: Final[TypeAdapter[list[RowsCase]]] = TypeAdapter(list[RowsCase])
OVERVIEW_CASES: Final[TypeAdapter[list[OverviewCase]]] = TypeAdapter(list[OverviewCase])


def window_answer(case: WindowCase) -> JsonValue:
    usage.datetime = FixedClock(datetime.fromisoformat(case.now))  # type: ignore[attr-defined,assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/usage_endpoints.py#L53
    try:
        start, end = WINDOW(case.start_date, case.end_date)
    except StarletteHTTPException as error:
        failure: dict[str, int] = {"status": error.status_code}
        return JSON.validate_python(failure)
    window: dict[str, str] = {"start": start, "end": end}
    return JSON.validate_python(window)


def threshold_answer(case: ThresholdCase) -> JsonValue:
    status = STATUS(case.fail_rate)
    trend = TREND(case.fail_rate, case.previous)
    thresholds: dict[str, str] = {"status": status, "trend": trend}
    return JSON.validate_python(thresholds)


def daily_answer(case: DailyCase) -> JsonValue:
    if case.reduce == "aggregate":
        aggregate: dict[str, MetricTotals] = dict(AGGREGATE(case.rows, METRIC_ID))
        return JSON.validate_python(aggregate)
    if case.reduce == "previous":
        previous: dict[str, float] = dict(PREVIOUS(case.rows, METRIC_ID))
        return JSON.validate_python(previous)
    charts: list[UsageChartPoint] = CHART(case.rows)
    return JSON.validate_python(charts)


def sum_answer(case: SumCase) -> JsonValue:
    if case.reduce == "units":
        units: dict[str, int] = dict(SUM_UNITS(case.rows))
        return JSON.validate_python(units)
    if case.reduce == "untracked":
        untracked: dict[str, int] = dict(SUM_UNTRACKED(case.rows))
        return JSON.validate_python(untracked)
    return JSON.validate_python(SUM_COST(case.rows))


def rows_answer(case: RowsCase) -> JsonValue:
    parts = [{"untracked": ROW_UNTRACKED(row), "cost": ROW_COST(row)} for row in case.rows]
    return JSON.validate_python(parts)


def group_answer(case: GroupCase) -> JsonValue:
    key = KEYS[case.key]
    if case.reduce == "units":
        grouped = BY_UNITS(case.rows, key, SUM_UNITS)
    elif case.reduce == "untracked":
        grouped = BY_UNTRACKED(case.rows, key, SUM_UNTRACKED)
    else:
        costs: dict[str, float | None] = dict(BY_COST(case.rows, key, SUM_COST))
        return JSON.validate_python(costs)
    counts: dict[str, dict[str, int]] = {name: dict(values) for name, values in grouped.items()}
    return JSON.validate_python(counts)


def guardrail_answer(case: OverviewCase) -> JsonValue:
    rows = GUARDRAIL_ROWS(
        [GuardrailRecord.model_validate(record) for record in case.records],
        case.aggregates.metrics,
        case.aggregates.previous_rates,
        case.aggregates.units,
        case.aggregates.costs,
        case.aggregates.untracked,
    )
    return JSON.validate_json(ROWS.dump_json(rows))


def policy_answer(case: OverviewCase) -> JsonValue:
    rows = POLICY_ROWS(
        [PolicyRow.model_validate(record) for record in case.records],
        case.aggregates.metrics,
        case.aggregates.previous_rates,
    )
    return JSON.validate_json(ROWS.dump_json(rows))


def answers(
    cases: Sequence[JsonObject],
    adapter: TypeAdapter[list[Case]],
    answer: Callable[[Case], JsonValue],
) -> JsonValue:
    results = [answer(case) for case in adapter.validate_python(cases)]
    payload: dict[str, list[JsonValue]] = {"results": results}
    return JSON.validate_python(payload)


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    cases = input_value["cases"]
    if function == "resolve_window":
        return answers(cases, WINDOW_CASES, window_answer)
    if function == "thresholds":
        return answers(cases, THRESHOLD_CASES, threshold_answer)
    if function in ("daily_metrics", "row_sums", "row_parts", "group_by"):
        return unit_answer(function, cases)
    return overview_answer(function, cases)


def unit_answer(function: str, cases: Sequence[JsonObject]) -> JsonValue:
    if function == "daily_metrics":
        return answers(cases, DAILY_CASES, daily_answer)
    if function == "row_sums":
        return answers(cases, SUM_CASES, sum_answer)
    if function == "row_parts":
        return answers(cases, ROWS_CASES, rows_answer)
    return answers(cases, GROUP_CASES, group_answer)


def overview_answer(function: str, cases: Sequence[JsonObject]) -> JsonValue:
    if function == "guardrail_overview":
        return answers(cases, OVERVIEW_CASES, guardrail_answer)
    if function == "policy_overview":
        return answers(cases, OVERVIEW_CASES, policy_answer)
    raise ValueError(f"unknown helper function {function}")
