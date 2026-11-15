from collections.abc import Sequence
from dataclasses import dataclass
from typing import Protocol, Self

from pydantic import JsonValue
from typing_extensions import override


@dataclass(frozen=True, slots=True, kw_only=True)
class PrometheusLabelSettings:
    custom_prometheus_metadata_labels: Sequence[str] = ()  # ast-grep-ignore: no-long-names
    custom_prometheus_tags: Sequence[str] = ()
    prometheus_emit_stream_label: bool = False
    prometheus_emit_rate_limit_labels: bool = False  # ast-grep-ignore: no-long-names
    prometheus_emit_input_sequence_length_label: bool = False  # ast-grep-ignore: no-long-names
    prometheus_user_budget_label_include_email_alias: bool = False  # ast-grep-ignore: no-long-names


@dataclass
class PrometheusMetricsConfig:
    group: str
    metrics: list[str]
    include_labels: list[str] | None = None


@dataclass
class PrometheusSettings:
    prometheus_metrics_config: list[PrometheusMetricsConfig] | None = None


class MetricLike(Protocol):
    def labels(self, *_args: JsonValue, **_kwargs: JsonValue) -> Self: ...

    def inc(self, *_args: JsonValue, **_kwargs: JsonValue) -> None: ...

    def set(self, *_args: JsonValue, **_kwargs: JsonValue) -> None: ...

    def observe(self, *_args: JsonValue, **_kwargs: JsonValue) -> None: ...


class NoOpMetric(MetricLike):
    def __init__(self, *_args: JsonValue, **_kwargs: JsonValue) -> None: ...

    @override
    def labels(self, *_args: JsonValue, **_kwargs: JsonValue) -> Self:
        return self

    @override
    def inc(self, *_args: JsonValue, **_kwargs: JsonValue) -> None: ...

    @override
    def set(self, *_args: JsonValue, **_kwargs: JsonValue) -> None: ...

    @override
    def observe(self, *_args: JsonValue, **_kwargs: JsonValue) -> None: ...


__all__ = [
    "MetricLike",
    "NoOpMetric",
    "PrometheusLabelSettings",
    "PrometheusMetricsConfig",
    "PrometheusSettings",
]
