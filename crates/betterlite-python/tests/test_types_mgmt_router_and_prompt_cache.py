import importlib
from typing import TypeAlias

from litellm import types as litellm_types
from pydantic import BaseModel, ConfigDict, JsonValue, ValidationError

import betterlite.types.management_endpoints.auto_router_endpoints as auto_router
import betterlite.types.management_endpoints.prompt_cache_prediction as prompt_cache
import betterlite.types.management_endpoints.prompt_caching_requests as prompt_caching

# litellm/types has no __init__.py, so pyright flags direct imports; load the modules by name
for module_name in (
    "litellm.types.management_endpoints.auto_router_endpoints",
    "litellm.types.management_endpoints.prompt_cache_prediction",
    "litellm.types.management_endpoints.prompt_caching_requests",
):
    importlib.import_module(module_name)

litellm_router = litellm_types.management_endpoints.auto_router_endpoints
litellm_cache = litellm_types.management_endpoints.prompt_cache_prediction
litellm_caching = litellm_types.management_endpoints.prompt_caching_requests

ModelPair: TypeAlias = tuple[type[BaseModel], type[BaseModel]]
ModelPayload: TypeAlias = tuple[type[BaseModel], type[BaseModel], dict[str, JsonValue]]

MODEL_PAIRS: tuple[ModelPair, ...] = (
    (
        auto_router.ComplexityRouterConfigValidationRequest,
        litellm_router.ComplexityRouterConfigValidationRequest,
    ),
    (
        auto_router.ComplexityRouterConfigValidationResponse,
        litellm_router.ComplexityRouterConfigValidationResponse,
    ),
    (auto_router.AutoRouterAvailabilityRequest, litellm_router.AutoRouterAvailabilityRequest),
    (auto_router.AutoRouterAllowance, litellm_router.AutoRouterAllowance),
    (auto_router.AutoRouterAvailabilityResponse, litellm_router.AutoRouterAvailabilityResponse),
    (auto_router.AutoRouterRoutingTestRequest, litellm_router.AutoRouterRoutingTestRequest),
    (auto_router.AutoRouterRoutingTestResponse, litellm_router.AutoRouterRoutingTestResponse),
    (auto_router.AutoRouterCacheBucket, litellm_router.AutoRouterCacheBucket),
    (auto_router.AutoRouterCacheStats, litellm_router.AutoRouterCacheStats),
    (auto_router.AutoRouterBenchmarkTotals, litellm_router.AutoRouterBenchmarkTotals),
    (auto_router.AutoRouterBenchmarkGroup, litellm_router.AutoRouterBenchmarkGroup),
    (auto_router.AutoRouterSessionResponse, litellm_router.AutoRouterSessionResponse),
    (auto_router.AutoRouterBenchmarksResponse, litellm_router.AutoRouterBenchmarksResponse),
    (auto_router.StartShadowEvalRequest, litellm_router.StartShadowEvalRequest),
    (auto_router.ShadowEvalSlice, litellm_router.ShadowEvalSlice),
    (auto_router.ShadowEvalResult, litellm_router.ShadowEvalResult),
    (auto_router.ShadowEvalJobTargetResponse, litellm_router.ShadowEvalJobTargetResponse),
    (auto_router.ShadowEvalJobResponse, litellm_router.ShadowEvalJobResponse),
    (prompt_cache.CacheTokenBuckets, litellm_cache.CacheTokenBuckets),
    (prompt_cache.CacheEvidence, litellm_cache.CacheEvidence),
    (prompt_cache.CacheCostScenario, litellm_cache.CacheCostScenario),
    (prompt_cache.CachePredictionArm, litellm_cache.CachePredictionArm),
    (prompt_cache.CachePredictionRequest, litellm_cache.CachePredictionRequest),
    (prompt_cache.CachePredictionResponse, litellm_cache.CachePredictionResponse),
    (prompt_caching.PromptCachingRequest, litellm_caching.PromptCachingRequest),
    (prompt_caching.PromptCachingRequestCursor, litellm_caching.PromptCachingRequestCursor),
    (prompt_caching.PromptCachingRequestsResponse, litellm_caching.PromptCachingRequestsResponse),
)

# these differ by JsonValue-for-object or computed fields, so their schemas are not compared
DIFFERENT_TYPES: tuple[ModelPair, ...] = (
    (
        auto_router.ComplexityRouterConfigValidationRequest,
        litellm_router.ComplexityRouterConfigValidationRequest,
    ),
    (auto_router.AutoRouterAvailabilityRequest, litellm_router.AutoRouterAvailabilityRequest),
    (auto_router.AutoRouterRoutingTestRequest, litellm_router.AutoRouterRoutingTestRequest),
    (auto_router.AutoRouterRoutingTestResponse, litellm_router.AutoRouterRoutingTestResponse),
    (auto_router.ShadowEvalJobResponse, litellm_router.ShadowEvalJobResponse),
)
SCHEMA_PAIRS: tuple[ModelPair, ...] = tuple(
    pair for pair in MODEL_PAIRS if pair not in DIFFERENT_TYPES
)

CACHE_BUCKET: dict[str, JsonValue] = {"turns": 0, "hits": 0, "hit_rate_pct": 0.0}
CACHE_STATS: dict[str, JsonValue] = {
    "coverage_pct": 0.0,
    "hit_rate_pct": 0.0,
    "same_model": CACHE_BUCKET,
    "first_visit": CACHE_BUCKET,
    "return_to_tier": CACHE_BUCKET,
    "unordered_turns": 0,
    "return_misses_expired": 0,
    "return_misses_within_ttl": 0,
    "return_misses_unknown": 0,
    "ttl_5m_turns": 0,
    "ttl_1h_turns": 0,
}
TOTALS: dict[str, JsonValue] = {
    "sessions": 0,
    "turns": 0,
    "avg_turns_per_session": 0.0,
    "avg_session_seconds": 0.0,
    "avg_tokens_per_session": 0.0,
    "spend": 0.0,
    "classifier_cost": None,
    "savings_estimated_turns": 0,
    "savings_estimated_actual_spend": 0.0,
    "saved_spend": None,
    "baseline_spend": None,
    "saved_pct": None,
    "saved_per_session": None,
    "cache": CACHE_STATS,
}
SLICE: dict[str, JsonValue] = {
    "group": "g",
    "turn_count": 0,
    "real_win_rate_pct": 0.0,
    "shadow_win_rate_pct": 0.0,
    "tie_rate_pct": 0.0,
    "avg_judge_confidence": 0.0,
}
JOB_TARGET: dict[str, JsonValue] = {"target_type": "key", "target_id": "k", "max_turns": 10}
SPENT_TARGET: dict[str, JsonValue] = JOB_TARGET | {"attempt_count": 10}
JOB: dict[str, JsonValue] = {
    "job_id": "j",
    "targets": [JOB_TARGET],
    "router_names": ["r"],
    "judge_model": "anthropic/claude-sonnet-5",
    "shadow_percentage": 50.0,
    "created_at": "2026-01-01T00:00:00Z",
    "ends_at": "2999-01-01T00:00:00Z",
}
MESSAGES: JsonValue = [{"role": "user", "content": "hi"}]
MESSAGES_ONLY: dict[str, JsonValue] = {"messages": MESSAGES, "complexity_router_config": {}}
BUCKETS: dict[str, JsonValue] = {
    "uncached_input_tokens": 1,
    "cache_read_input_tokens": 2,
    "cache_creation_5m_input_tokens": 3,
    "cache_creation_1h_input_tokens": 4,
}
BUCKET_TOTAL = 10
EVIDENCE: dict[str, JsonValue] = {"observed_at": 0.0, "expires_at": 1.0}
SCENARIO: dict[str, JsonValue] = {"tokens": BUCKETS, "input_cost": 0.5}
ARM: dict[str, JsonValue] = {
    "deployment_id": "d",
    "model": "m",
    "cache_state": "warm",
    "reason": "r",
    "estimate": SCENARIO,
    "cold": SCENARIO,
    "warm": SCENARIO,
    "evidence": EVIDENCE,
    "token_count_source": "anthropic_count_tokens",
}
CACHED_REQUEST: dict[str, JsonValue] = {
    "request_id": "r",
    "start_time": "2026-01-01T00:00:00Z",
    "model": "m",
    "gateway_injected": False,
    "cache_read_tokens": 0,
    "cache_creation_tokens": 0,
    "spend": 0.0,
    "net_savings": None,
}
CURSOR: dict[str, JsonValue] = {"start_time": "2026-01-01T00:00:00Z", "request_id": "r"}

DUMPS: tuple[ModelPayload, ...] = (
    (
        auto_router.ComplexityRouterConfigValidationRequest,
        litellm_router.ComplexityRouterConfigValidationRequest,
        {"complexity_router_config": {}},
    ),
    (
        auto_router.ComplexityRouterConfigValidationResponse,
        litellm_router.ComplexityRouterConfigValidationResponse,
        {"valid": True},
    ),
    (auto_router.AutoRouterAvailabilityRequest, litellm_router.AutoRouterAvailabilityRequest, {}),
    (
        auto_router.AutoRouterAllowance,
        litellm_router.AutoRouterAllowance,
        {"key": "k", "limit": None, "remaining": None},
    ),
    (
        auto_router.AutoRouterAvailabilityResponse,
        litellm_router.AutoRouterAvailabilityResponse,
        {"allowances": []},
    ),
    (
        auto_router.AutoRouterRoutingTestResponse,
        litellm_router.AutoRouterRoutingTestResponse,
        {
            "routed_model": "m",
            "routed_model_configured": True,
            "routing_decision": {"router_type": "complexity"},
        },
    ),
    (
        auto_router.AutoRouterCacheBucket,
        litellm_router.AutoRouterCacheBucket,
        {"turns": 1, "hits": 1, "hit_rate_pct": 100.0},
    ),
    (auto_router.AutoRouterCacheStats, litellm_router.AutoRouterCacheStats, CACHE_STATS),
    (auto_router.AutoRouterBenchmarkTotals, litellm_router.AutoRouterBenchmarkTotals, TOTALS),
    (
        auto_router.AutoRouterBenchmarkGroup,
        litellm_router.AutoRouterBenchmarkGroup,
        TOTALS | {"router_name": "r", "router_type": "complexity"},
    ),
    (
        auto_router.AutoRouterSessionResponse,
        litellm_router.AutoRouterSessionResponse,
        {
            "session_id": "s",
            "router_name": "r",
            "router_type": "complexity",
            "turns": 0,
            "last_model": "m",
            "spend": 0.0,
            "savings_estimated_turns": 0,
            "savings_estimated_actual_spend": 0.0,
            "saved_spend": None,
            "baseline_spend": None,
            "savings_estimated_baseline_spend": None,
            "baseline_model": None,
            "baseline_models": {},
        },
    ),
    (
        auto_router.AutoRouterBenchmarksResponse,
        litellm_router.AutoRouterBenchmarksResponse,
        {
            "start_date": "2026-01-01",
            "end_date": "2026-01-01",
            "routers_in_scope": 0,
            "totals": TOTALS,
            "groups": [],
        },
    ),
    (
        auto_router.StartShadowEvalRequest,
        litellm_router.StartShadowEvalRequest,
        {"api_key_ids": ["k"], "router_names": ["r"], "shadow_percentage": 50.0},
    ),
    (auto_router.ShadowEvalSlice, litellm_router.ShadowEvalSlice, SLICE),
    (
        auto_router.ShadowEvalResult,
        litellm_router.ShadowEvalResult,
        {
            "by_tier": [],
            "by_current_model": [],
            "overall_shadow_win_rate_pct": 0.0,
            "overall_tie_rate_pct": 0.0,
        },
    ),
    (
        auto_router.ShadowEvalJobTargetResponse,
        litellm_router.ShadowEvalJobTargetResponse,
        JOB_TARGET,
    ),
    (prompt_cache.CacheTokenBuckets, litellm_cache.CacheTokenBuckets, {}),
    (
        prompt_cache.CacheEvidence,
        litellm_cache.CacheEvidence,
        {"observed_at": 0.0, "expires_at": 1.0},
    ),
    (prompt_cache.CacheCostScenario, litellm_cache.CacheCostScenario, SCENARIO),
    (prompt_cache.CachePredictionArm, litellm_cache.CachePredictionArm, ARM),
    (
        prompt_cache.CachePredictionRequest,
        litellm_cache.CachePredictionRequest,
        {"current_deployment_id": "a", "candidate_deployment_id": "b", "request": {}},
    ),
    (
        prompt_cache.CachePredictionResponse,
        litellm_cache.CachePredictionResponse,
        {"stay": ARM, "switch": ARM, "switch_delta": None, "cache_rebuild_penalty": None},
    ),
    (prompt_caching.PromptCachingRequest, litellm_caching.PromptCachingRequest, CACHED_REQUEST),
    (prompt_caching.PromptCachingRequestCursor, litellm_caching.PromptCachingRequestCursor, CURSOR),
    (
        prompt_caching.PromptCachingRequestsResponse,
        litellm_caching.PromptCachingRequestsResponse,
        {"requests": [CACHED_REQUEST], "page_size": 1, "has_more": False, "next_cursor": None},
    ),
)

REJECTIONS: tuple[ModelPayload, ...] = (
    (
        auto_router.StartShadowEvalRequest,
        litellm_router.StartShadowEvalRequest,
        {"api_key_ids": ["k"], "router_names": ["r"], "shadow_percentage": 0.0},
    ),
    (
        auto_router.StartShadowEvalRequest,
        litellm_router.StartShadowEvalRequest,
        {
            "api_key_ids": ["k"],
            "router_names": ["r"],
            "shadow_percentage": 50.0,
            "duration_days": 0,
        },
    ),
    (
        auto_router.ShadowEvalJobTargetResponse,
        litellm_router.ShadowEvalJobTargetResponse,
        {"target_type": "org", "target_id": "k", "max_turns": 10},
    ),
    (
        auto_router.ShadowEvalJobResponse,
        litellm_router.ShadowEvalJobResponse,
        dict(JOB) | {"targets": []},
    ),
    (
        auto_router.ShadowEvalJobResponse,
        litellm_router.ShadowEvalJobResponse,
        dict(JOB) | {"router_names": []},
    ),
    (
        auto_router.AutoRouterRoutingTestRequest,
        litellm_router.AutoRouterRoutingTestRequest,
        {
            "messages": [{"role": "user", "content": "hi"}],
            "complexity_router_config": {},
            "saved_model_id": "",
        },
    ),
    (
        prompt_cache.CacheTokenBuckets,
        litellm_cache.CacheTokenBuckets,
        {"uncached_input_tokens": -1},
    ),
    (
        prompt_cache.CacheTokenBuckets,
        litellm_cache.CacheTokenBuckets,
        {"uncached_input_tokens": 1.0},
    ),
    (prompt_cache.CacheTokenBuckets, litellm_cache.CacheTokenBuckets, {"extra": 1}),
    (
        prompt_cache.CachePredictionArm,
        litellm_cache.CachePredictionArm,
        {"deployment_id": "d", "cache_state": "warmish"},
    ),
    (
        prompt_cache.CachePredictionArm,
        litellm_cache.CachePredictionArm,
        {"deployment_id": "d", "token_count_source": "bogus"},
    ),
    (
        prompt_cache.CachePredictionRequest,
        litellm_cache.CachePredictionRequest,
        {"current_deployment_id": "", "candidate_deployment_id": "b", "request": {}},
    ),
    (
        prompt_cache.CachePredictionRequest,
        litellm_cache.CachePredictionRequest,
        {"current_deployment_id": "a", "candidate_deployment_id": "b", "request": {}, "x": 1},
    ),
)


class FilterCarrier(BaseModel):
    value: prompt_caching.PromptCachingRequestFilter


class ReferenceFilterCarrier(BaseModel):
    value: litellm_caching.PromptCachingRequestFilter


class StatusCarrier(BaseModel):
    value: auto_router.ShadowEvalStatus


class ReferenceStatusCarrier(BaseModel):
    value: litellm_router.ShadowEvalStatus


AliasPayload: TypeAlias = tuple[type[BaseModel], type[BaseModel], tuple[str, ...], tuple[str, ...]]

ALIAS_VALUES: tuple[AliasPayload, ...] = (
    (FilterCarrier, ReferenceFilterCarrier, ("all", "injected", "hits"), ("bogus",)),
    (StatusCarrier, ReferenceStatusCarrier, ("running", "completed", "stopped"), ("bogus",)),
)


def dump(model: BaseModel) -> dict[str, JsonValue]:
    dumped: dict[str, JsonValue] = model.model_dump(mode="json")
    return dumped


def field_defaults(model: type[BaseModel]) -> list[tuple[str, JsonValue]]:
    defaults: list[tuple[str, JsonValue]] = []
    for name, field in model.model_fields.items():
        default: JsonValue = field.default
        defaults.append((name, default))
    return defaults


def frozen_policy(model: type[BaseModel]) -> str:
    config: ConfigDict = model.model_config
    if "frozen" not in config:
        return "absent"
    return str(config["frozen"])


def extra_policy(model: type[BaseModel]) -> str:
    config: ConfigDict = model.model_config
    if "extra" not in config:
        return "absent"
    value = config["extra"]
    return value if value is not None else "none"


def schema_of(model: type[BaseModel]) -> JsonValue:
    schema: dict[str, JsonValue] = model.model_json_schema(mode="serialization")
    return schema


def without_text(value: JsonValue) -> JsonValue:
    match value:
        case dict():
            return {
                key: without_text(item)
                for key, item in value.items()
                if key not in ("description", "title")
            }
        case list():
            return [without_text(item) for item in value]
        case None | str() | int() | float():
            return value


def rejects(model: type[BaseModel], payload: dict[str, JsonValue]) -> bool:
    try:
        model.model_validate(payload)
    except ValidationError:
        return True
    else:
        return False


def with_field(name: str, value: JsonValue) -> dict[str, JsonValue]:
    payload: dict[str, JsonValue] = dict(JOB)
    payload[name] = value
    return payload


def test_names_field_keys() -> None:
    for ours, reference in MODEL_PAIRS:
        assert list(ours.model_fields) == list(reference.model_fields), ours.__name__


def test_names_required_keys() -> None:
    for ours, reference in MODEL_PAIRS:
        ours_required = [name for name, field in ours.model_fields.items() if field.is_required()]
        required = [name for name, field in reference.model_fields.items() if field.is_required()]
        assert ours_required == required, ours.__name__


def test_names_field_defaults() -> None:
    for ours, reference in MODEL_PAIRS:
        assert field_defaults(ours) == field_defaults(reference), ours.__name__


def test_names_schema() -> None:
    for ours, reference in SCHEMA_PAIRS:
        ours_schema = without_text(schema_of(ours))
        reference_schema = without_text(schema_of(reference))
        assert ours_schema == reference_schema, ours.__name__


def test_names_complexity_config() -> None:
    # ComplexityRouterConfig has no Python owner at this pin; only the fields declared here port
    ours = auto_router.RequestComplexityRouterConfig.model_fields
    reference = litellm_router.RequestComplexityRouterConfig.model_fields
    assert list(ours) == ["plugins", "classifier_plugin"]
    for name, field in ours.items():
        default: JsonValue = field.default
        reference_default: JsonValue = reference[name].default
        assert default is None
        assert reference_default is None
    ours_config = auto_router.RequestComplexityRouterConfig.model_config
    reference_config = litellm_router.RequestComplexityRouterConfig.model_config
    assert ours_config.get("arbitrary_types_allowed") is True
    assert reference_config.get("arbitrary_types_allowed") is True
    assert extra_policy(auto_router.RequestComplexityRouterConfig) == "allow"
    assert extra_policy(litellm_router.RequestComplexityRouterConfig) == "allow"


def test_names_constants() -> None:
    assert (
        auto_router.DEFAULT_ROUTING_TEST_ROUTER_NAME
        == litellm_router.DEFAULT_ROUTING_TEST_ROUTER_NAME
    )
    assert (
        auto_router.DEFAULT_SHADOW_EVAL_JUDGE_MODEL
        == litellm_router.DEFAULT_SHADOW_EVAL_JUDGE_MODEL
    )
    assert auto_router.SHADOW_EVAL_TURN_VALVE == litellm_router.SHADOW_EVAL_TURN_VALVE
    assert auto_router.SHADOW_EVAL_MAX_ROUTERS == litellm_router.SHADOW_EVAL_MAX_ROUTERS


def test_names_alias_values() -> None:
    for ours_carrier, reference_carrier, accepted, rejected in ALIAS_VALUES:
        for value in accepted:
            assert not rejects(ours_carrier, {"value": value}), value
            assert not rejects(reference_carrier, {"value": value}), value
        for value in rejected:
            assert rejects(ours_carrier, {"value": value}), value
            assert rejects(reference_carrier, {"value": value}), value


def test_names_dumps() -> None:
    for ours, reference, payload in DUMPS:
        ours_dump = dump(ours.model_validate(payload))
        reference_dump = dump(reference.model_validate(payload))
        assert ours_dump == reference_dump, ours.__name__


def test_names_rejections() -> None:
    for ours, reference, payload in REJECTIONS:
        assert rejects(ours, payload) == rejects(reference, payload), ours.__name__


def test_names_validators_deferred() -> None:
    # B345 and B349 own the request validators; the models ported here run none of them
    shadow: tuple[dict[str, JsonValue], ...] = (
        {"shadow_percentage": 50.0},
        {"api_key_ids": ["k"], "shadow_percentage": 50.0, "max_turns": 10},
    )
    for payload in shadow:
        assert not rejects(auto_router.StartShadowEvalRequest, payload)
        assert rejects(litellm_router.StartShadowEvalRequest, payload)
    routing: tuple[dict[str, JsonValue], ...] = (
        {"messages": [], "complexity_router_config": {}},
        {"messages": [{"content": "hi"}], "complexity_router_config": {}},
        {"prompt": "  ", "complexity_router_config": {}},
    )
    for payload in routing:
        assert not rejects(auto_router.AutoRouterRoutingTestRequest, payload)
        assert rejects(litellm_router.AutoRouterRoutingTestRequest, payload)


def test_names_routing_test_request() -> None:  # ast-grep-ignore: no-long-names
    ours = auto_router.AutoRouterRoutingTestRequest.model_validate(MESSAGES_ONLY)
    reference = litellm_router.AutoRouterRoutingTestRequest.model_validate(MESSAGES_ONLY)
    ours_config: dict[str, JsonValue] = ours.complexity_router_config.model_dump()
    assert ours_config == {"plugins": None, "classifier_plugin": None}
    ours_dump = dump(ours)
    reference_dump = dump(reference)
    ours_dump["complexity_router_config"] = None
    reference_dump["complexity_router_config"] = None
    assert ours_dump == reference_dump


def test_names_wire_body() -> None:
    extras: dict[str, JsonValue] = {"system": "s", "tools": [{"type": "function"}]}
    full: dict[str, JsonValue] = MESSAGES_ONLY | extras
    full_body: dict[str, JsonValue] = {"messages": MESSAGES} | extras
    cases: tuple[tuple[dict[str, JsonValue], dict[str, JsonValue]], ...] = (
        (full, full_body),
        (MESSAGES_ONLY, {"messages": MESSAGES}),
    )
    for payload, wanted in cases:
        ours = auto_router.AutoRouterRoutingTestRequest.model_validate(payload)
        reference = litellm_router.AutoRouterRoutingTestRequest.model_validate(payload)
        assert dict(ours.wire_body()) == dict(reference.wire_body()) == wanted


def test_names_properties() -> None:
    target_payloads: tuple[dict[str, JsonValue], ...] = (
        {"target_type": "key", "target_id": "k", "max_turns": 10},
        {"target_type": "key", "target_id": "k", "max_turns": 10, "attempt_count": 10},
        {"target_type": "key", "target_id": "k", "max_turns": 10, "max_budget": 1.0, "spend": 2.0},
        {
            "target_type": "key",
            "target_id": "k",
            "max_turns": 10,
            "max_budget": 2.0,
            "spend": 1.0,
            "attempt_count": 9,
        },
        {"target_type": "key", "target_id": "k", "max_turns": 10, "max_budget": 1.0},
    )
    target_expected = (False, True, True, False, False)
    for payload, wanted in zip(target_payloads, target_expected, strict=True):
        ours_target = auto_router.ShadowEvalJobTargetResponse.model_validate(payload)
        reference_target = litellm_router.ShadowEvalJobTargetResponse.model_validate(payload)
        assert ours_target.budget_spent == reference_target.budget_spent == wanted
    job_payloads: tuple[dict[str, JsonValue], ...] = (
        JOB,
        with_field("stopped_by", "op"),
        with_field("ends_at", "2020-01-01T00:00:00Z"),
        with_field("targets", [SPENT_TARGET]),
        with_field("targets", [JOB_TARGET | {"stopped_at": "2024-01-01T00:00:00Z"}]),
        with_field("ends_at", "2999-01-01T00:00:00"),
    )
    job_expected = ("running", "stopped", "completed", "completed", "stopped", "running")
    for job_payload, job_wanted in zip(job_payloads, job_expected, strict=True):
        ours_job = auto_router.ShadowEvalJobResponse.model_validate(job_payload)
        reference_job = litellm_router.ShadowEvalJobResponse.model_validate(job_payload)
        assert ours_job.status == reference_job.status == job_wanted


def test_names_job_computed() -> None:
    # mypy strict rejects computed_field over a property, so both stay plain and out of the dump
    ours = auto_router.ShadowEvalJobResponse.model_validate(JOB)
    reference = litellm_router.ShadowEvalJobResponse.model_validate(JOB)
    assert ours.router_name == reference.router_name
    assert ours.status == reference.status
    ours_dump = dump(ours)
    reference_dump = dump(reference)
    reference_name: JsonValue = reference_dump.pop("router_name")
    reference_status: JsonValue = reference_dump.pop("status")
    assert reference_name == ours.router_name
    assert reference_status == ours.status
    assert "router_name" not in ours_dump
    assert "status" not in ours_dump
    assert ours_dump == reference_dump


def test_names_total_tokens() -> None:
    ours = prompt_cache.CacheTokenBuckets.model_validate(BUCKETS)
    reference = litellm_cache.CacheTokenBuckets.model_validate(BUCKETS)
    assert ours.total_tokens == reference.total_tokens == BUCKET_TOTAL


def test_names_model_config() -> None:
    for ours, reference in MODEL_PAIRS:
        assert frozen_policy(ours) == frozen_policy(reference), ours.__name__
        assert extra_policy(ours) == extra_policy(reference), ours.__name__
