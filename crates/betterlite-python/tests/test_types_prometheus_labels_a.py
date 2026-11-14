from decimal import Decimal
from typing import Final, get_args

import litellm
import litellm.integrations.prometheus as litellm_prometheus
import pytest
from pydantic import JsonValue

import betterlite.types.integrations.prometheus.core as our_core
from betterlite.types.integrations.prometheus.core import (
    _sanitize_prometheus_label_name,
    _sanitize_prometheus_label_value,
)
from betterlite.types.integrations.prometheus.labels import UserAPIKeyLabelNames
from betterlite.types.integrations.prometheus.validation import (
    CallerIdentitySettings,
    LabelValidationError,
    MetricValidationError,
    ValidationResults,
    _resolve_deployment_and_latency_caller_identity_labels,
    validate_caller_identity_settings,
    validate_prometheus_deployment_and_latency_caller_identity,
)

LABEL_NAME_COUNT: Final = 33
METRIC_NAME_COUNT: Final = 83
MODE_NAMES: Final[tuple[str, ...]] = ("api_key_alias", "user_email", "both")
IDENTITY_METRIC: Final = "litellm_deployment_total_requests"
OTHER_METRIC: Final = "litellm_spend_metric"
LATENCY_METRIC: Final = "litellm_llm_api_latency_metric"
BUCKET_INPUT_NAMES: Final[tuple[JsonValue | Decimal, ...]] = (
    0,
    1,
    999,
    1_000,
    1_001,
    3_999,
    4_000,
    4_001,
    15_999,
    16_000,
    16_001,
    63_999,
    64_000,
    64_001,
    1_000_000_000,
    Decimal(5),
    -1,
    True,
    False,
    0.5,
    1_000.0,
    "500",
    None,
)
# Recorded from litellm/types/integrations/prometheus.py at d665dc1a6315847fcad6cc2c1264ceb47c06abce
SANITIZED_LABEL_NAMES: Final[list[tuple[str, str]]] = [
    ("", "_"),
    ("simple", "simple"),
    ("with space", "with_space"),
    ("with.dot", "with_dot"),
    ("with-dash", "with_dash"),
    ("1leading", "_1leading"),
    ("0", "_0"),
    ("-lead", "_lead"),
    ("\u00fc\u00f1\u00ee", "___"),
    ("a:b/c", "a_b_c"),
    ("a b.c-d", "a_b_c_d"),
    ("_", "_"),
    ("  ", "__"),
    ("tag_User-Agent: curl/7.68.0", "tag_User_Agent__curl_7_68_0"),
    ("x" * 40 + " y", "x" * 40 + "_y"),
]
SANITIZED_LABEL_VALUES: Final[list[tuple[JsonValue, str | None]]] = [
    (None, None),
    ("", ""),
    ("plain", "plain"),
    ("with space", "with space"),
    ("with.dot", "with.dot"),
    ("with-dash", "with-dash"),
    ('quote"inside', 'quote\\"inside'),
    ("back\\slash", "back\\\\slash"),
    ("line\nbreak", "line break"),
    ("carriage\rreturn", "carriagereturn"),
    ("sep\u2028line", "sepline"),
    ("sep\u2029line", "sepline"),
    ("\u00fc\u00f1\u00ee", "\u00fc\u00f1\u00ee"),
    ('\\"both', '\\\\\\"both'),
    (1, "1"),
    (0, "0"),
    (1.5, "1.5"),
    (True, "True"),
    ("\n\r", " "),
]
# Recorded from the same LiteLLM file: the resolver for the caller-identity metrics
RESOLVED_LABEL_NAMES: Final[tuple[tuple[str, list[str]], ...]] = (
    ("api_key_alias", ["team", "api_key_alias", "model"]),
    ("user_email", ["team", "user_email", "model"]),
    ("both", ["team", "api_key_alias", "user_email", "model"]),
)
CALLER_IDENTITY_LABELS: Final[list[str]] = ["team", "api_key_alias", "model"]
REJECTED_LABEL_SETS: Final[list[tuple[str, list[JsonValue | UserAPIKeyLabelNames]]]] = [
    (IDENTITY_METRIC, ["team", 7]),
    (IDENTITY_METRIC, [UserAPIKeyLabelNames.TEAM, "api_key_alias"]),
    (OTHER_METRIC, [UserAPIKeyLabelNames.TEAM, "api_key_alias"]),
]
CONFLICTING_NAMES_MESSAGE: Final = (
    "prometheus_metrics_config include_labels contains 'api_key_alias' for "
    "litellm_deployment_total_requests, litellm_llm_api_latency_metric, but "
    "prometheus_deployment_and_latency_caller_identity='user_email' replaces that "
    "label on these metrics. Use 'user_email' in include_labels or change the mode."
)
CONFLICTING_SETTINGS_NAMES: Final[list[tuple[CallerIdentitySettings, str | None]]] = [
    (
        {
            "prometheus_deployment_and_latency_caller_identity": "both",
            "prometheus_metrics_config": [
                {"metrics": [IDENTITY_METRIC], "include_labels": ["api_key_alias"]},
            ],
        },
        None,
    ),
    (
        {
            "prometheus_deployment_and_latency_caller_identity": "user_email",
            "prometheus_metrics_config": [
                {"metrics": [OTHER_METRIC], "include_labels": ["api_key_alias"]},
            ],
        },
        None,
    ),
    (
        {
            "prometheus_deployment_and_latency_caller_identity": "user_email",
            "prometheus_metrics_config": [
                {
                    "metrics": [IDENTITY_METRIC, OTHER_METRIC, LATENCY_METRIC],
                    "include_labels": ["api_key_alias"],
                },
            ],
        },
        CONFLICTING_NAMES_MESSAGE,
    ),
    (
        {
            "prometheus_deployment_and_latency_caller_identity": "user_email",
            "prometheus_metrics_config": {
                "metrics": [IDENTITY_METRIC],
                "include_labels": ["api_key_alias"],
            },
        },
        None,
    ),
    (
        {
            "prometheus_deployment_and_latency_caller_identity": "user_email",
            "prometheus_metrics_config": [OTHER_METRIC],
        },
        None,
    ),
    (
        {
            "prometheus_deployment_and_latency_caller_identity": "user_email",
            "prometheus_metrics_config": [
                {"metrics": [IDENTITY_METRIC], "include_labels": 0},
            ],
        },
        None,
    ),
    (
        {
            "prometheus_deployment_and_latency_caller_identity": "user_email",
            "prometheus_metrics_config": [
                {"metrics": 0, "include_labels": ["api_key_alias"]},
            ],
        },
        None,
    ),
]


@pytest.fixture(autouse=True)
def _reset_caller_identity_names() -> None:
    validate_caller_identity_settings(
        {"prometheus_deployment_and_latency_caller_identity": "api_key_alias"},
    )
    litellm_prometheus.validate_caller_identity_settings(
        {"prometheus_deployment_and_latency_caller_identity": "api_key_alias"},
    )


# ast-grep-ignore: no-long-names
def test_names_metric_catalog_matches_litellm() -> None:
    their_names: tuple[str, ...] = get_args(litellm_prometheus.DEFINED_PROMETHEUS_METRICS)
    our_names: tuple[str, ...] = get_args(our_core.DEFINED_PROMETHEUS_METRICS)
    assert our_names == their_names
    assert len(our_names) == METRIC_NAME_COUNT


# ast-grep-ignore: no-long-names
def test_names_constants_match_litellm() -> None:
    assert our_core.REQUESTED_MODEL == litellm_prometheus.REQUESTED_MODEL
    assert our_core.EXCEPTION_STATUS == litellm_prometheus.EXCEPTION_STATUS
    assert our_core.EXCEPTION_CLASS == litellm_prometheus.EXCEPTION_CLASS
    assert our_core.RATE_LIMIT_CATEGORY == litellm_prometheus.RATE_LIMIT_CATEGORY
    assert our_core.RATE_LIMIT_TYPE == litellm_prometheus.RATE_LIMIT_TYPE
    assert our_core.ZERO_COST_REASON_LABEL == litellm_prometheus.ZERO_COST_REASON_LABEL
    assert our_core.STATUS_CODE == litellm_prometheus.STATUS_CODE
    assert our_core.EXCEPTION_LABELS == litellm_prometheus.EXCEPTION_LABELS
    assert our_core.LATENCY_BUCKETS == litellm_prometheus.LATENCY_BUCKETS
    assert our_core.UNKNOWN_INPUT_SEQUENCE_LENGTH == (
        litellm_prometheus.UNKNOWN_INPUT_SEQUENCE_LENGTH
    )
    assert our_core.INPUT_SEQUENCE_LENGTH_BUCKETS == (
        litellm_prometheus.INPUT_SEQUENCE_LENGTH_BUCKETS
    )
    assert our_core.BATCH_DURATION_BUCKETS == litellm_prometheus.BATCH_DURATION_BUCKETS


# ast-grep-ignore: no-long-names
def test_names_enum_members_match_litellm() -> None:
    their_members = [
        (member.name, member.value) for member in litellm_prometheus.UserAPIKeyLabelNames
    ]
    our_members = [(member.name, member.value) for member in UserAPIKeyLabelNames]
    assert our_members == their_members
    assert len(our_members) == LABEL_NAME_COUNT
    assert UserAPIKeyLabelNames.REQUESTED_MODEL.value == our_core.REQUESTED_MODEL
    assert UserAPIKeyLabelNames.EXCEPTION_STATUS.value == our_core.EXCEPTION_STATUS
    assert UserAPIKeyLabelNames.EXCEPTION_CLASS.value == our_core.EXCEPTION_CLASS
    assert UserAPIKeyLabelNames.RATE_LIMIT_CATEGORY.value == our_core.RATE_LIMIT_CATEGORY
    assert UserAPIKeyLabelNames.RATE_LIMIT_TYPE.value == our_core.RATE_LIMIT_TYPE
    assert UserAPIKeyLabelNames.v1_LITELLM_MODEL_NAME.value == "model"
    assert UserAPIKeyLabelNames.v2_LITELLM_MODEL_NAME.value == "litellm_model_name"


# ast-grep-ignore: no-long-names
def test_names_caller_identity_metric_set_matches_litellm() -> None:
    assert (
        our_core.PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_METRICS
        == litellm_prometheus.PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_METRICS
    )
    assert (
        our_core.PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_VALUES
        == litellm_prometheus.PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_VALUES
    )


@pytest.mark.parametrize("prompt_tokens", BUCKET_INPUT_NAMES)
# ast-grep-ignore: no-long-names
def test_names_input_sequence_length_bucket_matches_litellm(
    prompt_tokens: JsonValue | Decimal,
) -> None:
    theirs = litellm_prometheus.get_input_sequence_length_bucket(prompt_tokens)
    ours = our_core.get_input_sequence_length_bucket(prompt_tokens)
    assert ours == theirs


@pytest.mark.parametrize(("label", "sanitized"), SANITIZED_LABEL_NAMES)
# ast-grep-ignore: no-long-names
def test_names_sanitized_label_name_matches_litellm(label: str, sanitized: str) -> None:
    ours = _sanitize_prometheus_label_name(label)
    assert ours == sanitized


@pytest.mark.parametrize(("value", "sanitized"), SANITIZED_LABEL_VALUES)
# ast-grep-ignore: no-long-names
def test_names_sanitized_label_value_matches_litellm(
    value: JsonValue,
    sanitized: str | None,
) -> None:
    ours = _sanitize_prometheus_label_value(value)
    assert ours == sanitized


@pytest.mark.parametrize("mode", MODE_NAMES)
# ast-grep-ignore: no-long-names
def test_names_caller_identity_validator_matches_litellm(mode: str) -> None:
    validate_caller_identity_settings({"prometheus_deployment_and_latency_caller_identity": mode})
    litellm_prometheus.validate_caller_identity_settings(
        {"prometheus_deployment_and_latency_caller_identity": mode},
    )
    ours = validate_prometheus_deployment_and_latency_caller_identity()
    theirs = litellm.prometheus_deployment_and_latency_caller_identity
    assert ours == theirs


@pytest.mark.parametrize("mode", MODE_NAMES)
# ast-grep-ignore: no-long-names
def test_names_validation_messages_match_litellm(mode: str) -> None:
    validate_caller_identity_settings({"prometheus_deployment_and_latency_caller_identity": mode})
    litellm_prometheus.validate_caller_identity_settings(
        {"prometheus_deployment_and_latency_caller_identity": mode},
    )
    our_metric_error = MetricValidationError(
        metric_name="litellm_nope",
        valid_metrics=("litellm_spend_metric",),
    )
    their_metric_error = litellm_prometheus.MetricValidationError(
        metric_name="litellm_nope",
        valid_metrics=("litellm_spend_metric",),
    )
    assert our_metric_error.message == their_metric_error.message
    our_identity_error = LabelValidationError(
        metric_name=IDENTITY_METRIC,
        invalid_labels=["api_key_alias", "team"],
        valid_labels=["user_email"],
    )
    their_identity_error = litellm_prometheus.LabelValidationError(
        metric_name=IDENTITY_METRIC,
        invalid_labels=["api_key_alias", "team"],
        valid_labels=["user_email"],
    )
    assert our_identity_error.message == their_identity_error.message
    our_other_metric_error = LabelValidationError(
        metric_name=OTHER_METRIC,
        invalid_labels=["api_key_alias"],
        valid_labels=["user_email"],
    )
    their_other_metric_error = litellm_prometheus.LabelValidationError(
        metric_name=OTHER_METRIC,
        invalid_labels=["api_key_alias"],
        valid_labels=["user_email"],
    )
    assert our_other_metric_error.message == their_other_metric_error.message
    our_other_label_error = LabelValidationError(
        metric_name=IDENTITY_METRIC,
        invalid_labels=["team"],
        valid_labels=["user_email"],
    )
    their_other_label_error = litellm_prometheus.LabelValidationError(
        metric_name=IDENTITY_METRIC,
        invalid_labels=["team"],
        valid_labels=["user_email"],
    )
    assert our_other_label_error.message == their_other_label_error.message


# ast-grep-ignore: no-long-names
def test_names_dataclass_fields_match_litellm() -> None:
    our_metric_error = MetricValidationError(metric_name="m", valid_metrics=("a", "b"))
    their_metric_error = litellm_prometheus.MetricValidationError(
        metric_name="m",
        valid_metrics=("a", "b"),
    )
    assert our_metric_error.metric_name == their_metric_error.metric_name
    assert our_metric_error.valid_metrics == their_metric_error.valid_metrics
    our_label_error = LabelValidationError(
        metric_name="m",
        invalid_labels=["a"],
        valid_labels=["b"],
    )
    their_label_error = litellm_prometheus.LabelValidationError(
        metric_name="m",
        invalid_labels=["a"],
        valid_labels=["b"],
    )
    assert our_label_error.metric_name == their_label_error.metric_name
    assert our_label_error.invalid_labels == their_label_error.invalid_labels
    assert our_label_error.valid_labels == their_label_error.valid_labels
    our_results = ValidationResults(
        metric_errors=[our_metric_error],
        label_errors=[our_label_error],
    )
    their_results = litellm_prometheus.ValidationResults(
        metric_errors=[their_metric_error],
        label_errors=[their_label_error],
    )
    assert len(our_results.metric_errors) == len(their_results.metric_errors)
    assert len(our_results.label_errors) == len(their_results.label_errors)
    assert our_results.all_error_messages == their_results.all_error_messages


# ast-grep-ignore: no-long-names
def test_names_validation_results_match_litellm() -> None:
    our_metric_error = MetricValidationError(metric_name="m", valid_metrics=())
    their_metric_error = litellm_prometheus.MetricValidationError(
        metric_name="m",
        valid_metrics=(),
    )
    our_label_error = LabelValidationError(
        metric_name=OTHER_METRIC,
        invalid_labels=["team"],
        valid_labels=[],
    )
    their_label_error = litellm_prometheus.LabelValidationError(
        metric_name=OTHER_METRIC,
        invalid_labels=["team"],
        valid_labels=[],
    )
    cases = [
        (
            ValidationResults(metric_errors=[], label_errors=[]),
            litellm_prometheus.ValidationResults(metric_errors=[], label_errors=[]),
        ),
        (
            ValidationResults(metric_errors=[our_metric_error], label_errors=[]),
            litellm_prometheus.ValidationResults(
                metric_errors=[their_metric_error],
                label_errors=[],
            ),
        ),
        (
            ValidationResults(metric_errors=[], label_errors=[our_label_error]),
            litellm_prometheus.ValidationResults(
                metric_errors=[],
                label_errors=[their_label_error],
            ),
        ),
        (
            ValidationResults(metric_errors=[our_metric_error], label_errors=[our_label_error]),
            litellm_prometheus.ValidationResults(
                metric_errors=[their_metric_error],
                label_errors=[their_label_error],
            ),
        ),
    ]
    for ours, theirs in cases:
        assert ours.has_errors == theirs.has_errors
        assert ours.all_error_messages == theirs.all_error_messages


@pytest.mark.parametrize(("mode", "expected"), RESOLVED_LABEL_NAMES)
# ast-grep-ignore: no-long-names
def test_names_caller_identity_resolution_matches_litellm(mode: str, expected: list[str]) -> None:
    validate_caller_identity_settings({"prometheus_deployment_and_latency_caller_identity": mode})
    labels = list(CALLER_IDENTITY_LABELS)
    resolved = _resolve_deployment_and_latency_caller_identity_labels(IDENTITY_METRIC, labels)
    assert resolved == expected
    assert labels == CALLER_IDENTITY_LABELS
    other = _resolve_deployment_and_latency_caller_identity_labels(OTHER_METRIC, labels)
    assert other == CALLER_IDENTITY_LABELS


@pytest.mark.parametrize(("metric_name", "labels"), REJECTED_LABEL_SETS)
# ast-grep-ignore: no-long-names
def test_names_caller_identity_resolution_rejects_non_string_labels(
    metric_name: str,
    labels: list[JsonValue | UserAPIKeyLabelNames],
) -> None:
    with pytest.raises(TypeError, match="must be strings"):
        _resolve_deployment_and_latency_caller_identity_labels(metric_name, labels)


# ast-grep-ignore: no-long-names
def test_names_caller_identity_resolution_needs_the_alias_label() -> None:
    with pytest.raises(ValueError, match="'api_key_alias' is not in list"):
        _resolve_deployment_and_latency_caller_identity_labels(IDENTITY_METRIC, ["team"])


# ast-grep-ignore: no-long-names
def test_names_invalid_caller_identity_settings_match_litellm() -> None:
    with pytest.raises(
        ValueError,
        match="Invalid prometheus_deployment_and_latency_caller_identity",
    ) as their_error:
        litellm_prometheus.validate_caller_identity_settings(
            {"prometheus_deployment_and_latency_caller_identity": 5},
        )
    with pytest.raises(
        ValueError,
        match="Invalid prometheus_deployment_and_latency_caller_identity",
    ) as our_error:
        validate_caller_identity_settings(
            {"prometheus_deployment_and_latency_caller_identity": 5},
        )
    assert str(our_error.value) == str(their_error.value)


@pytest.mark.parametrize(("settings", "expected_message"), CONFLICTING_SETTINGS_NAMES)
# ast-grep-ignore: no-long-names
def test_names_conflicting_caller_identity_settings_match_litellm(
    settings: CallerIdentitySettings,
    expected_message: str | None,
) -> None:
    if expected_message is None:
        validate_caller_identity_settings(settings)
        litellm_prometheus.validate_caller_identity_settings(settings)
        return
    with pytest.raises(ValueError, match=expected_message) as our_error:
        validate_caller_identity_settings(settings)
    with pytest.raises(ValueError, match=expected_message) as their_error:
        litellm_prometheus.validate_caller_identity_settings(settings)
    assert str(our_error.value) == str(their_error.value)
