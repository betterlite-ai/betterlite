from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from types import MappingProxyType
from typing import Final, Literal

from typing_extensions import ReadOnly, TypedDict

from betterlite.types.integrations.custom_logger import StandardCustomLoggerInitParams


class NewRelicInitParams(StandardCustomLoggerInitParams):
    pass


# ast-grep-ignore: no-long-names
NEWRELIC_METRIC_ENDPOINT_BY_REGION: Final[Mapping[str, str]] = MappingProxyType(
    {
        "us": "https://metric-api.newrelic.com/metric/v1",
        "eu": "https://metric-api.eu.newrelic.com/metric/v1",
    }
)

NEWRELIC_DEFAULT_REGION: Final = "us"

# ast-grep-ignore: no-long-names
NEWRELIC_METRICS_MAX_BATCH_SIZE: Final = 200

# ast-grep-ignore: no-long-names
NEWRELIC_METRICS_MAX_RETRY_QUEUE_SIZE: Final = 10_000
# ast-grep-ignore: no-long-names
NEWRELIC_METRICS_MAX_DRAIN_PASSES: Final = 3
# ast-grep-ignore: no-long-names
NEWRELIC_METRIC_ATTRIBUTE_MAX_LEN: Final = 255

NEWRELIC_METRIC_REQUESTS: Final = "litellm.requests"
NEWRELIC_METRIC_COST_USD: Final = "litellm.cost.usd"
NEWRELIC_METRIC_PROMPT_TOKENS: Final = "litellm.tokens.prompt"
# ast-grep-ignore: no-long-names
NEWRELIC_METRIC_COMPLETION_TOKENS: Final = "litellm.tokens.completion"
NEWRELIC_METRIC_TOTAL_TOKENS: Final = "litellm.tokens.total"
# ast-grep-ignore: no-long-names
NEWRELIC_METRIC_REQUEST_DURATION_MS: Final = "litellm.request.duration_ms"
# ast-grep-ignore: no-long-names
NEWRELIC_METRIC_TEAM_MAX_BUDGET: Final = "litellm.team.max_budget"
# ast-grep-ignore: no-long-names
NEWRELIC_METRIC_TEAM_REMAINING_BUDGET: Final = "litellm.team.remaining_budget"


class NewRelicSummaryValue(TypedDict):
    count: ReadOnly[int]
    sum: ReadOnly[float]
    min: ReadOnly[float]
    max: ReadOnly[float]


class NewRelicCountMetric(TypedDict):
    name: ReadOnly[str]
    type: ReadOnly[Literal["count"]]
    value: ReadOnly[float]
    attributes: ReadOnly[Mapping[str, str]]


class NewRelicGaugeMetric(TypedDict):
    name: ReadOnly[str]
    type: ReadOnly[Literal["gauge"]]
    value: ReadOnly[float]
    attributes: ReadOnly[Mapping[str, str]]


class NewRelicSummaryMetric(TypedDict):
    name: ReadOnly[str]
    type: ReadOnly[Literal["summary"]]
    value: ReadOnly[NewRelicSummaryValue]
    attributes: ReadOnly[Mapping[str, str]]


NewRelicMetric = NewRelicCountMetric | NewRelicGaugeMetric | NewRelicSummaryMetric

NewRelicMetricCommon = TypedDict(
    "NewRelicMetricCommon",
    {
        "timestamp": ReadOnly[int],
        "interval.ms": ReadOnly[int],
    },
)


class NewRelicMetricEnvelope(TypedDict):
    common: ReadOnly[NewRelicMetricCommon]
    metrics: ReadOnly[Sequence[NewRelicMetric]]


@dataclass(frozen=True, slots=True)
class NewRelicMetricRecord:
    team_id: str
    team_alias: str
    model_group: str
    model: str
    custom_llm_provider: str
    status: str
    response_cost: float
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    duration_ms: float
    team_max_budget: float | None = None
    team_spend: float | None = None

    @property
    def bucket_key(self) -> tuple[str, str, str, str, str, str]:
        return (
            self.team_id,
            self.team_alias,
            self.model_group,
            self.model,
            self.custom_llm_provider,
            self.status,
        )
