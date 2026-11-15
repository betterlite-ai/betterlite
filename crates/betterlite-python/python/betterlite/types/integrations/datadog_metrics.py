from typing_extensions import TypedDict


class DatadogMetricPoint(TypedDict):
    timestamp: int
    value: float


class DatadogMetricSeries(TypedDict, total=False):
    metric: str
    type: int
    points: list[DatadogMetricPoint]
    tags: list[str]
    interval: int | None


class DatadogMetricsPayload(TypedDict):
    series: list[DatadogMetricSeries]
