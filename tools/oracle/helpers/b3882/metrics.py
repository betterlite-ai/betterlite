import asyncio
from datetime import datetime
from typing import Final, TypedDict

from litellm.integrations.prometheus import PrometheusLogger
from litellm.integrations.prometheus_helpers.bounded_prometheus_series_tracker import (
    BoundedPrometheusSeriesTracker,
)
from litellm.models.team import LiteLLM_TeamTable, Member
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import UserAPIKeyAuth
from litellm.types.integrations.prometheus import DEFINED_PROMETHEUS_METRICS
from prometheus_client import CollectorRegistry, Gauge
from typing_extensions import override

from .inputs import (
    ALL_METRICS,
    CustomerCall,
    CustomerRow,
    CustomerRowCall,
    EntityCall,
    KeyListCall,
    KeyListEntry,
    OrganizationRow,
    OrgListCall,
    ProviderCall,
    RemainingBudgetCall,
    RemainingHoursCall,
    TeamCall,
    TeamMembersCall,
    UserCall,
    budget_row,
    parse_optional,
)


class SampleRow(TypedDict):
    metric: str
    labels: dict[str, str]
    value: str


def full_labels(provided: dict[str, list[str]]) -> dict[str, list[str]]:
    labels: dict[str, list[str]] = {metric: [] for metric in ALL_METRICS}
    labels.update(provided)
    return labels


def sample_sort_key(row: SampleRow) -> tuple[str, list[tuple[str, str]]]:
    labels = sorted(row["labels"].items())
    return (row["metric"], labels)


def key_entry(entry: KeyListEntry) -> UserAPIKeyAuth | str:
    if entry["kind"] == "token":
        return entry["token"]
    return UserAPIKeyAuth(
        token=entry["token"],
        key_alias=entry["key_alias"],
        spend=entry["spend"],
        max_budget=entry["max_budget"],
        budget_reset_at=parse_optional(entry["budget_reset_at"]),
    )


class ProbeLogger(PrometheusLogger):
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
        return Gauge(
            metric,
            metric,
            labelnames=self._labels_by_metric[metric],
            registry=self._registry,
        )

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

    def entity(self, call: EntityCall) -> None:
        if call["function"] == "team":
            self._set_team_budget_metrics(
                LiteLLM_TeamTable(
                    team_id=call["team_id"],
                    team_alias=call["team_alias"],
                    spend=call["spend"],
                    max_budget=call["max_budget"],
                    budget_reset_at=parse_optional(call["budget_reset_at"]),
                )
            )
        elif call["function"] == "org":
            self._set_org_budget_metrics(
                org_id=call["org_id"],
                org_alias=call["org_alias"],
                spend=call["spend"],
                max_budget=call["max_budget"],
                budget_reset_at=parse_optional(call["budget_reset_at"]),
            )
        elif call["function"] == "key":
            self._set_key_budget_metrics(
                UserAPIKeyAuth(
                    token=call["token"],
                    key_alias=call["key_alias"],
                    spend=call["spend"],
                    max_budget=call["max_budget"],
                    budget_reset_at=parse_optional(call["budget_reset_at"]),
                )
            )
        else:
            self._set_user_budget_metrics(
                LiteLLM_UserTable(
                    user_id=call["user_id"],
                    user_email=call["user_email"],
                    user_alias=call["user_alias"],
                    spend=call["spend"],
                    max_budget=call["max_budget"],
                    budget_reset_at=parse_optional(call["budget_reset_at"]),
                )
            )

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
        keys = [key_entry(entry) for entry in call["entries"]]
        asyncio.run(self._set_key_list_budget_metrics(keys))

    def org_list(self, call: OrgListCall) -> None:
        rows = [
            OrganizationRow(
                row["organization_id"],
                row["organization_alias"],
                row["spend"],
                budget_row(row["budget"]),
            )
            for row in call["rows"]
        ]
        asyncio.run(self._set_org_list_budget_metrics(rows))  # type: ignore[misc]  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L3893

    def customer_row(self, call: CustomerRowCall) -> None:
        customer = CustomerRow(
            call["customer"]["user_id"],
            call["customer"]["spend"],
            budget_row(call["customer"]["budget"]),
        )
        self._set_customer_budget_metrics_from_row(customer, budget_row(call["default_budget"]))

    def samples(self) -> list[SampleRow]:
        # a gauge the call never labels reports zero, so only the metrics the call names are read
        rows: list[SampleRow] = [
            {
                "metric": sample.name,
                "labels": dict(sample.labels),
                "value": format(sample.value, ".6f"),
            }
            for family in self._registry.collect()
            if family.name in self._requested
            for sample in family.samples
        ]
        rows.sort(key=sample_sort_key)
        return rows
