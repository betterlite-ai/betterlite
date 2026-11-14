from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from typing import ClassVar, TypedDict

from pydantic import JsonValue

from betterlite.types.integrations.prometheus.core import (
    PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_METRICS,
    PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_VALUES,
)
from betterlite.types.integrations.prometheus.labels import UserAPIKeyLabelNames


class _CallerIdentityState:
    # LiteLLM keeps the mode on its module and the resolvers read it at call time
    mode: ClassVar[JsonValue] = "api_key_alias"


@dataclass
class MetricValidationError:
    metric_name: str
    valid_metrics: tuple[str, ...]

    @property
    def message(self) -> str:
        return f"Invalid metric name: {self.metric_name}"


@dataclass
class LabelValidationError:
    metric_name: str
    invalid_labels: list[str]
    valid_labels: list[str]

    @property
    def message(self) -> str:
        base_message = f"Invalid labels for metric '{self.metric_name}': {self.invalid_labels}"
        # ast-grep-ignore: no-long-names
        if self.metric_name not in PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_METRICS:
            return base_message
        if not any(label in ("api_key_alias", "user_email") for label in self.invalid_labels):
            return base_message
        mode = _CallerIdentityState.mode
        return (
            f"{base_message} (the caller-identity label on this metric is set by "
            f"prometheus_deployment_and_latency_caller_identity={mode!r})"
        )


@dataclass
class ValidationResults:
    metric_errors: list[MetricValidationError]
    label_errors: list[LabelValidationError]

    @property
    def has_errors(self) -> bool:
        return bool(self.metric_errors) or bool(self.label_errors)

    @property
    def all_error_messages(self) -> list[str]:
        messages = [error.message for error in self.metric_errors]
        messages.extend(error.message for error in self.label_errors)
        return messages


class PrometheusMetricsConfigEntry(TypedDict, total=False):
    metrics: Sequence[str]
    include_labels: Sequence[str]


class CallerIdentitySettings(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    prometheus_deployment_and_latency_caller_identity: JsonValue
    prometheus_metrics_config: Sequence[PrometheusMetricsConfigEntry]


# ast-grep-ignore: no-long-names
def validate_prometheus_deployment_and_latency_caller_identity() -> str:
    mode = _CallerIdentityState.mode
    match mode:
        case "api_key_alias" | "user_email" | "both":
            return mode
        case _:
            # ast-grep-ignore: no-long-names
            accepted_values = ", ".join(PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_VALUES)
            raise ValueError(
                "Invalid prometheus_deployment_and_latency_caller_identity="
                f"{mode!r}. Accepted values: {accepted_values}."
            )


# ast-grep-ignore: no-long-names
def validate_caller_identity_settings(litellm_settings: CallerIdentitySettings) -> None:
    if "prometheus_deployment_and_latency_caller_identity" not in litellm_settings:
        return
    _CallerIdentityState.mode = litellm_settings[
        "prometheus_deployment_and_latency_caller_identity"
    ]
    caller_identity_mode = validate_prometheus_deployment_and_latency_caller_identity()
    if caller_identity_mode != "user_email":
        return
    metrics_config = litellm_settings.get("prometheus_metrics_config")
    if metrics_config is None:
        return
    conflicting_metrics: list[str] = []
    for metric_config in metrics_config:
        include_labels = metric_config.get("include_labels")
        if include_labels is None:
            continue
        if UserAPIKeyLabelNames.API_KEY_ALIAS.value not in include_labels:
            continue
        metrics = metric_config.get("metrics")
        if metrics is None:
            continue
        conflicting_metrics.extend(
            metric_name
            for metric_name in metrics
            # ast-grep-ignore: no-long-names
            if metric_name in PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_METRICS
        )
    if conflicting_metrics:
        conflicting_names = ", ".join(conflicting_metrics)
        raise ValueError(
            "prometheus_metrics_config include_labels contains 'api_key_alias' for "
            f"{conflicting_names}, but prometheus_deployment_and_latency_caller_identity="
            "'user_email' replaces that label on these metrics. Use 'user_email' in "
            "include_labels or change the mode."
        )


# ast-grep-ignore: no-long-names
def _resolve_deployment_and_latency_caller_identity_labels(
    metric_name: str,
    labels: Sequence[JsonValue],
) -> list[str]:
    resolved_labels: list[str] = []
    for label in labels:
        match label:
            case str():
                resolved_labels.append(label)
            case None | bool() | int() | float() | Mapping() | Sequence():
                raise TypeError(f"Prometheus labels for {metric_name} must be strings")
    # ast-grep-ignore: no-long-names
    if metric_name not in PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_METRICS:
        return resolved_labels
    caller_identity = validate_prometheus_deployment_and_latency_caller_identity()
    alias_index = resolved_labels.index(UserAPIKeyLabelNames.API_KEY_ALIAS.value)
    if caller_identity == "user_email":
        resolved_labels[alias_index] = UserAPIKeyLabelNames.USER_EMAIL.value
    elif caller_identity == "both":
        resolved_labels.insert(alias_index + 1, UserAPIKeyLabelNames.USER_EMAIL.value)
    return resolved_labels


__all__ = [
    "CallerIdentitySettings",
    "LabelValidationError",
    "MetricValidationError",
    "PrometheusMetricsConfigEntry",
    "ValidationResults",
    "_resolve_deployment_and_latency_caller_identity_labels",
    "validate_caller_identity_settings",
    "validate_prometheus_deployment_and_latency_caller_identity",
]
