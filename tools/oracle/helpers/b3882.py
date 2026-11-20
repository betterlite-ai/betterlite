import asyncio
from datetime import UTC, datetime
from typing import Final, Literal, TypedDict

import litellm
from litellm.integrations.prometheus import PrometheusLogger
from litellm.integrations.prometheus_helpers.bounded_prometheus_series_tracker import (
    BoundedPrometheusSeriesTracker,
)
from litellm.models.team import LiteLLM_TeamTable, Member
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import UserAPIKeyAuth
from litellm.types.integrations.prometheus import DEFINED_PROMETHEUS_METRICS
from prometheus_client import CollectorRegistry, Gauge
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

_JSON: Final = TypeAdapter(JsonValue)

# LiteLLM registers this gauge with labelnames=["api_provider"], so it is the one metric whose label list is fixed
PROVIDER_METRIC: Final = "litellm_provider_remaining_budget_metric"

ALL_METRICS: Final = (
    "litellm_remaining_team_budget_metric",
    "litellm_team_max_budget_metric",
    "litellm_team_budget_remaining_hours_metric",
    "litellm_team_members_metric",
    "litellm_remaining_org_budget_metric",
    "litellm_org_max_budget_metric",
    "litellm_org_budget_remaining_hours_metric",
    "litellm_remaining_api_key_budget_metric",
    "litellm_api_key_max_budget_metric",
    "litellm_api_key_budget_remaining_hours_metric",
    "litellm_remaining_user_budget_metric",
    "litellm_user_max_budget_metric",
    "litellm_user_budget_remaining_hours_metric",
    "litellm_remaining_customer_budget_metric",
    "litellm_customer_max_budget_metric",
    "litellm_customer_budget_remaining_hours_metric",
    "litellm_provider_remaining_budget_metric",
)


class RemainingBudgetCall(TypedDict):
    function: Literal["remaining_budget"]
    max_budget: float | None
    spend: float | None


class RemainingHoursCall(TypedDict):
    function: Literal["remaining_hours"]
    reset_at: str
    now: str


class TeamCall(TypedDict):
    function: Literal["team"]
    team_id: str
    team_alias: str | None
    spend: float | None
    max_budget: float | None
    budget_reset_at: str | None
    now: str
    labels_by_metric: dict[str, list[str]]


class OrgCall(TypedDict):
    function: Literal["org"]
    org_id: str
    org_alias: str
    spend: float
    max_budget: float | None
    budget_reset_at: str | None
    now: str
    labels_by_metric: dict[str, list[str]]


class KeyCall(TypedDict):
    function: Literal["key"]
    token: str
    key_alias: str | None
    spend: float | None
    max_budget: float | None
    budget_reset_at: str | None
    now: str
    labels_by_metric: dict[str, list[str]]


class UserCall(TypedDict):
    function: Literal["user"]
    user_id: str
    user_email: str | None
    user_alias: str | None
    spend: float
    max_budget: float | None
    budget_reset_at: str | None
    now: str
    labels_by_metric: dict[str, list[str]]


class CustomerCall(TypedDict):
    function: Literal["customer"]
    end_user_id: str
    spend: float
    max_budget: float | None
    budget_reset_at: str | None
    now: str
    enable_end_user_tracking: bool
    labels_by_metric: dict[str, list[str]]


class TeamMembersCall(TypedDict):
    function: Literal["team_members"]
    team_id: str
    team_alias: str | None
    member_count: int
    labels_by_metric: dict[str, list[str]]


class ProviderCall(TypedDict):
    function: Literal["provider"]
    provider: str
    spend: float
    budget_limit: float


class KeyListEntryRecord(TypedDict):
    kind: Literal["record"]
    token: str
    key_alias: str | None
    spend: float
    max_budget: float | None
    budget_reset_at: str | None


class KeyListEntryToken(TypedDict):
    kind: Literal["token"]
    token: str


KeyListEntry = KeyListEntryRecord | KeyListEntryToken


class KeyListCall(TypedDict):
    function: Literal["key_list"]
    entries: list[KeyListEntry]
    now: str
    labels_by_metric: dict[str, list[str]]


class BudgetRowInput(TypedDict):
    max_budget: float | None
    budget_reset_at: str | None


class OrgRowInput(TypedDict):
    organization_id: str | None
    organization_alias: str | None
    spend: float | None
    budget: BudgetRowInput | None


class OrgListCall(TypedDict):
    function: Literal["org_list"]
    rows: list[OrgRowInput]
    now: str
    labels_by_metric: dict[str, list[str]]


class CustomerRowInput(TypedDict):
    user_id: str
    spend: float
    budget: BudgetRowInput | None


class CustomerRowCall(TypedDict):
    function: Literal["customer_row"]
    customer: CustomerRowInput
    default_budget: BudgetRowInput | None
    now: str
    enable_end_user_tracking: bool
    labels_by_metric: dict[str, list[str]]


Input = (
    RemainingBudgetCall
    | RemainingHoursCall
    | TeamCall
    | OrgCall
    | KeyCall
    | UserCall
    | CustomerCall
    | TeamMembersCall
    | ProviderCall
    | KeyListCall
    | OrgListCall
    | CustomerRowCall
)


class SampleRow(TypedDict):
    metric: str
    labels: dict[str, str]
    value: str


class BudgetRow:
    def __init__(self, max_budget: float | None, budget_reset_at: datetime | None) -> None:
        self.max_budget = max_budget
        self.budget_reset_at = budget_reset_at


class OrganizationRow:
    def __init__(
        self,
        organization_id: str | None,
        organization_alias: str | None,
        spend: float | None,
        litellm_budget_table: BudgetRow | None,
    ) -> None:
        self.organization_id = organization_id
        self.organization_alias = organization_alias
        self.spend = spend
        self.litellm_budget_table = litellm_budget_table


class CustomerRow:
    def __init__(
        self,
        user_id: str,
        spend: float,
        litellm_budget_table: BudgetRow | None,
    ) -> None:
        self.user_id = user_id
        self.spend = spend
        self.litellm_budget_table = litellm_budget_table


def full_labels(provided: dict[str, list[str]]) -> dict[str, list[str]]:
    labels: dict[str, list[str]] = {metric: [] for metric in ALL_METRICS}
    labels.update(provided)
    return labels


def parse_optional(text: str | None) -> datetime | None:
    if text is None:
        return None
    moment = datetime.fromisoformat(text)
    return moment


def budget_row(call: BudgetRowInput | None) -> BudgetRow | None:
    if call is None:
        return None
    row = BudgetRow(call["max_budget"], parse_optional(call["budget_reset_at"]))
    return row


def sample_sort_key(row: SampleRow) -> tuple[str, list[tuple[str, str]]]:
    labels = sorted(row["labels"].items())
    return (row["metric"], labels)


class ProbeLogger(PrometheusLogger):
    litellm_remaining_team_budget_metric: Gauge
    litellm_team_max_budget_metric: Gauge
    litellm_team_budget_remaining_hours_metric: Gauge
    litellm_team_members_metric: Gauge
    litellm_remaining_org_budget_metric: Gauge
    litellm_org_max_budget_metric: Gauge
    litellm_org_budget_remaining_hours_metric: Gauge
    litellm_remaining_api_key_budget_metric: Gauge
    litellm_api_key_max_budget_metric: Gauge
    litellm_api_key_budget_remaining_hours_metric: Gauge
    litellm_remaining_user_budget_metric: Gauge
    litellm_user_max_budget_metric: Gauge
    litellm_user_budget_remaining_hours_metric: Gauge
    litellm_remaining_customer_budget_metric: Gauge
    litellm_customer_max_budget_metric: Gauge
    litellm_customer_budget_remaining_hours_metric: Gauge
    litellm_provider_remaining_budget_metric: Gauge

    def __init__(self, labels_by_metric: dict[str, list[str]], now: datetime) -> None:
        self._requested = frozenset(labels_by_metric)
        self._labels_by_metric = full_labels(labels_by_metric)
        self._now = now
        self._registry = CollectorRegistry()
        self._bounded_prometheus_series_tracker = BoundedPrometheusSeriesTracker()
        self.litellm_remaining_team_budget_metric = self._gauge(
            "litellm_remaining_team_budget_metric"
        )
        self.litellm_team_max_budget_metric = self._gauge("litellm_team_max_budget_metric")
        self.litellm_team_budget_remaining_hours_metric = self._gauge(
            "litellm_team_budget_remaining_hours_metric"
        )
        self.litellm_team_members_metric = self._gauge("litellm_team_members_metric")
        self.litellm_remaining_org_budget_metric = self._gauge(
            "litellm_remaining_org_budget_metric"
        )
        self.litellm_org_max_budget_metric = self._gauge("litellm_org_max_budget_metric")
        self.litellm_org_budget_remaining_hours_metric = self._gauge(
            "litellm_org_budget_remaining_hours_metric"
        )
        self.litellm_remaining_api_key_budget_metric = self._gauge(
            "litellm_remaining_api_key_budget_metric"
        )
        self.litellm_api_key_max_budget_metric = self._gauge("litellm_api_key_max_budget_metric")
        self.litellm_api_key_budget_remaining_hours_metric = self._gauge(
            "litellm_api_key_budget_remaining_hours_metric"
        )
        self.litellm_remaining_user_budget_metric = self._gauge(
            "litellm_remaining_user_budget_metric"
        )
        self.litellm_user_max_budget_metric = self._gauge("litellm_user_max_budget_metric")
        self.litellm_user_budget_remaining_hours_metric = self._gauge(
            "litellm_user_budget_remaining_hours_metric"
        )
        self.litellm_remaining_customer_budget_metric = self._gauge(
            "litellm_remaining_customer_budget_metric"
        )
        self.litellm_customer_max_budget_metric = self._gauge(
            "litellm_customer_max_budget_metric"
        )
        self.litellm_customer_budget_remaining_hours_metric = self._gauge(
            "litellm_customer_budget_remaining_hours_metric"
        )
        self.litellm_provider_remaining_budget_metric = self._gauge(
            "litellm_provider_remaining_budget_metric"
        )

    def _gauge(self, metric: str) -> Gauge:
        gauge = Gauge(
            metric,
            metric,
            labelnames=self._labels_by_metric[metric],
            registry=self._registry,
        )
        return gauge

    @override
    def get_labels_for_metric(self, metric_name: DEFINED_PROMETHEUS_METRICS) -> list[str]:
        return self._labels_by_metric[metric_name]

    @override
    def _get_remaining_hours_for_budget_reset(  # ast-grep-ignore: no-long-names
        self, budget_reset_at: datetime
    ) -> float:
        return (budget_reset_at - self._now).total_seconds() / 3600

    def remaining_budget(self, call: RemainingBudgetCall) -> float:
        return self._safe_get_remaining_budget(max_budget=call["max_budget"], spend=call["spend"])

    def remaining_hours(self, call: RemainingHoursCall) -> float:
        reset_at = datetime.fromisoformat(call["reset_at"])
        return self._get_remaining_hours_for_budget_reset(budget_reset_at=reset_at)

    def team(self, call: TeamCall) -> None:
        team = LiteLLM_TeamTable(
            team_id=call["team_id"],
            team_alias=call["team_alias"],
            spend=call["spend"],
            max_budget=call["max_budget"],
            budget_reset_at=parse_optional(call["budget_reset_at"]),
        )
        self._set_team_budget_metrics(team)

    def org(self, call: OrgCall) -> None:
        self._set_org_budget_metrics(
            org_id=call["org_id"],
            org_alias=call["org_alias"],
            spend=call["spend"],
            max_budget=call["max_budget"],
            budget_reset_at=parse_optional(call["budget_reset_at"]),
        )

    def key(self, call: KeyCall) -> None:
        key = UserAPIKeyAuth(
            token=call["token"],
            key_alias=call["key_alias"],
            spend=call["spend"],
            max_budget=call["max_budget"],
            budget_reset_at=parse_optional(call["budget_reset_at"]),
        )
        self._set_key_budget_metrics(key)

    def user(self, call: UserCall) -> None:
        user = LiteLLM_UserTable(
            user_id=call["user_id"],
            user_email=call["user_email"],
            user_alias=call["user_alias"],
            spend=call["spend"],
            max_budget=call["max_budget"],
            budget_reset_at=parse_optional(call["budget_reset_at"]),
        )
        self._set_user_budget_metrics(user)

    def customer(self, call: CustomerCall) -> None:
        self._set_customer_budget_metrics(
            end_user_id=call["end_user_id"],
            spend=call["spend"],
            max_budget=call["max_budget"],
            budget_reset_at=parse_optional(call["budget_reset_at"]),
        )

    def team_members(self, call: TeamMembersCall) -> None:
        members = [
            Member(user_id=f"member-{index}", role="user") for index in range(call["member_count"])
        ]
        team = LiteLLM_TeamTable(
            team_id=call["team_id"],
            team_alias=call["team_alias"],
            members_with_roles=members,
        )
        self.set_team_members_metric(team)

    def provider(self, call: ProviderCall) -> None:
        self.track_provider_remaining_budget(
            call["provider"],
            call["spend"],
            call["budget_limit"],
        )

    def key_list(self, call: KeyListCall) -> None:
        keys: list[str | UserAPIKeyAuth] = []
        for entry in call["entries"]:
            if entry["kind"] == "token":
                keys.append(entry["token"])
            else:
                keys.append(
                    UserAPIKeyAuth(
                        token=entry["token"],
                        key_alias=entry["key_alias"],
                        spend=entry["spend"],
                        max_budget=entry["max_budget"],
                        budget_reset_at=parse_optional(entry["budget_reset_at"]),
                    )
                )
        asyncio.run(self._set_key_list_budget_metrics(keys))

    def org_list(self, call: OrgListCall) -> None:
        rows: list[OrganizationRow] = []
        for row in call["rows"]:
            rows.append(
                OrganizationRow(
                    row["organization_id"],
                    row["organization_alias"],
                    row["spend"],
                    budget_row(row["budget"]),
                )
            )
        asyncio.run(self._set_org_list_budget_metrics(rows))

    def customer_row(self, call: CustomerRowCall) -> None:
        customer = CustomerRow(
            call["customer"]["user_id"],
            call["customer"]["spend"],
            budget_row(call["customer"]["budget"]),
        )
        self._set_customer_budget_metrics_from_row(customer, budget_row(call["default_budget"]))

    def samples(self) -> list[SampleRow]:
        # a gauge the call never labels still reports the zero value, so only the metrics the call names are read
        rows: list[SampleRow] = []
        for family in self._registry.collect():
            if family.name not in self._requested:
                continue
            for sample in family.samples:
                rows.append(
                    {
                        "metric": sample.name,
                        "labels": dict(sample.labels),
                        "value": format(sample.value, ".6f"),
                    }
                )
        rows.sort(key=sample_sort_key)
        return rows


def probe(labels_by_metric: dict[str, list[str]], now: str) -> ProbeLogger:
    logger = ProbeLogger(labels_by_metric, datetime.fromisoformat(now))
    return logger


def remaining_budget(call: RemainingBudgetCall) -> JsonValue:
    logger = ProbeLogger({}, datetime(1970, 1, 1, tzinfo=UTC))
    remaining = logger.remaining_budget(call)
    return _JSON.validate_python({"remaining": format(remaining, ".6f")})


def remaining_hours(call: RemainingHoursCall) -> JsonValue:
    logger = ProbeLogger({}, datetime.fromisoformat(call["now"]))
    hours = logger.remaining_hours(call)
    return _JSON.validate_python({"hours": format(hours, ".6f")})


def entity_output(logger: ProbeLogger) -> JsonValue:
    return _JSON.validate_python({"samples": logger.samples()})


def team(call: TeamCall) -> JsonValue:
    logger = probe(call["labels_by_metric"], call["now"])
    logger.team(call)
    return entity_output(logger)


def org(call: OrgCall) -> JsonValue:
    logger = probe(call["labels_by_metric"], call["now"])
    logger.org(call)
    return entity_output(logger)


def key(call: KeyCall) -> JsonValue:
    logger = probe(call["labels_by_metric"], call["now"])
    logger.key(call)
    return entity_output(logger)


def user(call: UserCall) -> JsonValue:
    logger = probe(call["labels_by_metric"], call["now"])
    logger.user(call)
    return entity_output(logger)


def customer(call: CustomerCall) -> JsonValue:
    litellm.disable_end_user_cost_tracking = False
    litellm.enable_end_user_cost_tracking_prometheus_only = call["enable_end_user_tracking"]
    logger = probe(call["labels_by_metric"], call["now"])
    logger.customer(call)
    return entity_output(logger)


def customer_row(call: CustomerRowCall) -> JsonValue:
    litellm.disable_end_user_cost_tracking = False
    litellm.enable_end_user_cost_tracking_prometheus_only = call["enable_end_user_tracking"]
    logger = probe(call["labels_by_metric"], call["now"])
    logger.customer_row(call)
    return entity_output(logger)


def team_members(call: TeamMembersCall) -> JsonValue:
    logger = ProbeLogger(call["labels_by_metric"], datetime(1970, 1, 1, tzinfo=UTC))
    logger.team_members(call)
    return entity_output(logger)


def provider(call: ProviderCall) -> JsonValue:
    labels = {PROVIDER_METRIC: ["api_provider"]}
    logger = ProbeLogger(labels, datetime(1970, 1, 1, tzinfo=UTC))
    logger.provider(call)
    return entity_output(logger)


def key_list(call: KeyListCall) -> JsonValue:
    logger = probe(call["labels_by_metric"], call["now"])
    logger.key_list(call)
    return entity_output(logger)


def org_list(call: OrgListCall) -> JsonValue:
    logger = probe(call["labels_by_metric"], call["now"])
    logger.org_list(call)
    return entity_output(logger)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "remaining_budget":
        output = remaining_budget(input_value)
    elif input_value["function"] == "remaining_hours":
        output = remaining_hours(input_value)
    elif input_value["function"] == "team":
        output = team(input_value)
    elif input_value["function"] == "org":
        output = org(input_value)
    elif input_value["function"] == "key":
        output = key(input_value)
    elif input_value["function"] == "user":
        output = user(input_value)
    elif input_value["function"] == "customer":
        output = customer(input_value)
    elif input_value["function"] == "team_members":
        output = team_members(input_value)
    elif input_value["function"] == "provider":
        output = provider(input_value)
    elif input_value["function"] == "key_list":
        output = key_list(input_value)
    elif input_value["function"] == "org_list":
        output = org_list(input_value)
    else:
        output = customer_row(input_value)
    return output
