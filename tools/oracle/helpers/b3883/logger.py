from datetime import datetime
from typing import TypedDict

from litellm.integrations.prometheus import PrometheusLogger
from litellm.integrations.prometheus_helpers.bounded_prometheus_series_tracker import (
    BoundedPrometheusSeriesTracker,
)
from litellm.models.budget import LiteLLM_BudgetTableFull
from litellm.proxy.utils import PrismaClient
from litellm.types.integrations.prometheus import DEFINED_PROMETHEUS_METRICS, NoOpMetric
from prometheus_client import CollectorRegistry, Gauge
from typing_extensions import override

from .inputs import (
    CREATED_AT,
    CustomerCall,
    KeyCall,
    OrgCall,
    TeamCall,
    UserCall,
    key_snapshot,
    org_snapshot,
    team_snapshot,
    user_snapshot,
)


class SampleRow(TypedDict):
    metric: str
    labels: dict[str, str]
    value: str


class BudgetLogger(PrometheusLogger):
    def __init__(
        self,
        labels_by_metric: dict[str, list[str]],
        now: datetime,
        *,
        noop: bool,
    ) -> None:
        self._requested = frozenset(labels_by_metric)
        self._metrics: dict[str, list[str]] = (
            {metric: [] for metric in _ALL_METRICS} | labels_by_metric
        )
        self._now = now
        self._noop = noop
        self._gauges: list[Gauge] = []
        self._registry = CollectorRegistry()
        self._bounded_prometheus_series_tracker = BoundedPrometheusSeriesTracker()
        self.litellm_remaining_team_budget_metric = self._budget_gauge(
            "litellm_remaining_team_budget_metric"
        )
        self.litellm_team_max_budget_metric = self._budget_gauge(
            "litellm_team_max_budget_metric"
        )
        self.litellm_team_budget_remaining_hours_metric = self._budget_gauge(
            "litellm_team_budget_remaining_hours_metric"
        )
        self.litellm_remaining_org_budget_metric = self._budget_gauge(
            "litellm_remaining_org_budget_metric"
        )
        self.litellm_org_max_budget_metric = self._budget_gauge(
            "litellm_org_max_budget_metric"
        )
        self.litellm_org_budget_remaining_hours_metric = self._budget_gauge(
            "litellm_org_budget_remaining_hours_metric"
        )
        self.litellm_remaining_api_key_budget_metric = self._budget_gauge(
            "litellm_remaining_api_key_budget_metric"
        )
        self.litellm_api_key_max_budget_metric = self._budget_gauge(
            "litellm_api_key_max_budget_metric"
        )
        self.litellm_api_key_budget_remaining_hours_metric = self._budget_gauge(
            "litellm_api_key_budget_remaining_hours_metric"
        )
        self.litellm_remaining_user_budget_metric = self._budget_gauge(
            "litellm_remaining_user_budget_metric"
        )
        self.litellm_user_max_budget_metric = self._budget_gauge(
            "litellm_user_max_budget_metric"
        )
        self.litellm_user_budget_remaining_hours_metric = self._budget_gauge(
            "litellm_user_budget_remaining_hours_metric"
        )
        self.litellm_remaining_customer_budget_metric = self._budget_gauge(
            "litellm_remaining_customer_budget_metric"
        )
        self.litellm_customer_max_budget_metric = self._budget_gauge(
            "litellm_customer_max_budget_metric"
        )
        self.litellm_customer_budget_remaining_hours_metric = self._budget_gauge(
            "litellm_customer_budget_remaining_hours_metric"
        )

    def _budget_gauge(self, metric: str) -> Gauge | NoOpMetric:
        if self._noop or metric not in self._requested:
            return NoOpMetric()
        gauge = Gauge(
            metric,
            metric,
            labelnames=self._metrics[metric],
            registry=self._registry,
        )
        self._gauges.append(gauge)
        return gauge

    @override
    def get_labels_for_metric(self, metric_name: DEFINED_PROMETHEUS_METRICS) -> list[str]:
        return self._metrics[metric_name]

    @override
    def _get_remaining_hours_for_budget_reset(  # ast-grep-ignore: no-long-names
        self, budget_reset_at: datetime
    ) -> float:
        return (budget_reset_at - self._now).total_seconds() / 3600

    async def team(self, call: TeamCall) -> None:
        await self._set_team_budget_metrics_after_api_request(
            user_api_team=call["team_id"],
            user_api_team_alias=call["team_alias"],
            team_spend=call["team_spend"],
            team_max_budget=call["team_max_budget"],
            response_cost=call["response_cost"],
            carried=team_snapshot(call["carried"]),
        )

    async def org(self, call: OrgCall) -> None:
        await self._set_org_budget_metrics_after_api_request(
            org_id=call["org_id"],
            response_cost=call["response_cost"],
            carried=org_snapshot(call["carried"]),
            org_alias=call["org_alias"],
        )

    async def customer(self, call: CustomerCall) -> None:
        await self._set_customer_budget_metrics_after_api_request(
            end_user_id=call["end_user_id"],
            response_cost=call["response_cost"],
        )

    async def key(self, call: KeyCall) -> None:
        await self._set_api_key_budget_metrics_after_api_request(
            user_api_key=call["user_api_key"],
            user_api_key_alias=call["user_api_key_alias"],
            response_cost=call["response_cost"],
            key_max_budget=call["key_max_budget"],
            key_spend=call["key_spend"],
            carried=key_snapshot(call["carried"]),
        )

    async def user(self, call: UserCall) -> None:
        await self._set_user_budget_metrics_after_api_request(
            user_id=call["user_id"],
            user_spend=call["user_spend"],
            user_max_budget=call["user_max_budget"],
            response_cost=call["response_cost"],
            carried=user_snapshot(call["carried"]),
            user_email=call["user_email"],
        )

    async def default_customer_budget(
        self, prisma_client: PrismaClient
    ) -> LiteLLM_BudgetTableFull | None:
        row = await self._get_default_customer_budget(prisma_client)
        if row is None:
            return None
        return LiteLLM_BudgetTableFull(
            max_budget=row.max_budget,
            budget_reset_at=row.budget_reset_at,
            created_at=CREATED_AT,
        )

    def samples(self) -> list[SampleRow]:
        # gauges sit in registration order, so samples keep the order the setter wrote them
        return [
            {
                "metric": sample.name,
                "labels": dict(sample.labels),
                "value": format(sample.value, ".6f"),
            }
            for gauge in self._gauges
            for family in gauge.collect()
            for sample in family.samples
        ]


_ALL_METRICS: tuple[str, ...] = (
    "litellm_remaining_team_budget_metric",
    "litellm_team_max_budget_metric",
    "litellm_team_budget_remaining_hours_metric",
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
)
