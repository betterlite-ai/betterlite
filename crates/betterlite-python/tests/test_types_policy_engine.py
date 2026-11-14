from typing import Protocol

import litellm.types.policy_engine as litellm_compat  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/policy_engine.py#L1
import litellm.types.proxy.policy_engine.pipeline_types as litellm_pipeline  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/policy_engine/pipeline_types.py#L1
import litellm.types.proxy.policy_engine.policy_types as litellm_policy  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/policy_engine/policy_types.py#L1
import litellm.types.proxy.policy_engine.resolver_types as litellm_resolver  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/policy_engine/resolver_types.py#L1
import litellm.types.proxy.policy_engine.validation_types as litellm_validation  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/policy_engine/validation_types.py#L1
import pytest
from pydantic import BaseModel, JsonValue, ValidationError

import betterlite.types.policy_engine as betterlite_compat
import betterlite.types.proxy.policy_engine.pipeline_types as betterlite_pipeline
import betterlite.types.proxy.policy_engine.policy_types as betterlite_policy
import betterlite.types.proxy.policy_engine.resolver_types as betterlite_resolver
import betterlite.types.proxy.policy_engine.validation_types as betterlite_validation


class ResolverModels(Protocol):
    PolicyCreateRequest: type[litellm_resolver.PolicyCreateRequest]
    PolicyUpdateRequest: type[litellm_resolver.PolicyUpdateRequest]
    PolicyDBResponse: type[litellm_resolver.PolicyDBResponse]
    PolicyVersionCompareResponse: type[litellm_resolver.PolicyVersionCompareResponse]
    PipelineTestRequest: type[litellm_resolver.PipelineTestRequest]


class PipelineModels(Protocol):
    PipelineStepResult: type[litellm_pipeline.PipelineStepResult]
    PipelineExecutionResult: type[litellm_pipeline.PipelineExecutionResult]


class ValidationModels(Protocol):
    PolicyValidateRequest: type[litellm_validation.PolicyValidateRequest]


resolver_models: ResolverModels = litellm_resolver
pipeline_models: PipelineModels = litellm_pipeline
validation_models: ValidationModels = litellm_validation


MODEL_PARAMS = [
    pytest.param(
        betterlite_policy.PolicyCondition, litellm_policy.PolicyCondition, {},
        id="PolicyCondition-defaults",
    ),
    pytest.param(
        betterlite_policy.PolicyCondition, litellm_policy.PolicyCondition,
        {"model": "gpt-4"}, id="PolicyCondition-model",
    ),
    pytest.param(
        betterlite_policy.PolicyCondition, litellm_policy.PolicyCondition,
        {"model": ["gpt-4", "gpt-4o"]}, id="PolicyCondition-models",
    ),
    pytest.param(
        betterlite_policy.PolicyScope, litellm_policy.PolicyScope, {},
        id="PolicyScope-defaults",
    ),
    pytest.param(
        betterlite_policy.PolicyScope, litellm_policy.PolicyScope,
        {"teams": ["healthcare-*"], "keys": ["dev-key-*"], "models": ["gpt-*"],
         "tags": ["prod-*"]},
        id="PolicyScope-values",
    ),
    pytest.param(
        betterlite_policy.PolicyGuardrails, litellm_policy.PolicyGuardrails, {},
        id="PolicyGuardrails-defaults",
    ),
    pytest.param(
        betterlite_policy.PolicyGuardrails, litellm_policy.PolicyGuardrails,
        {"add": ["pii_blocker"], "remove": ["phi_blocker"]}, id="PolicyGuardrails-values",
    ),
    pytest.param(betterlite_policy.Policy, litellm_policy.Policy, {}, id="Policy-defaults"),
    pytest.param(
        betterlite_policy.Policy, litellm_policy.Policy,
        {
            "inherit": "global-baseline",
            "description": "HIPAA compliance for healthcare",
            "guardrails": {"add": ["hipaa_audit"], "remove": ["phi_blocker"]},
            "condition": {"model": ["gpt-4", "gpt-4.*"]},
            "pipeline": {
                "mode": "pre_call",
                "steps": [{"guardrail": "hipaa_audit", "on_fail": "block"}],
            },
        },
        id="Policy-values",
    ),
    pytest.param(
        betterlite_policy.PolicyAttachment, litellm_policy.PolicyAttachment,
        {"policy": "global-baseline"}, id="PolicyAttachment-minimal",
    ),
    pytest.param(
        betterlite_policy.PolicyAttachment, litellm_policy.PolicyAttachment,
        {"policy": "global-baseline", "scope": "*"},
        id="PolicyAttachment-global",
    ),
    pytest.param(
        betterlite_policy.PolicyAttachment, litellm_policy.PolicyAttachment,
        {"policy": "dev-safety", "teams": ["healthcare-team"], "keys": ["dev-key-*"],
         "models": ["gpt-4"], "tags": ["health-*"], "priority": 3, "default": True},
        id="PolicyAttachment-values",
    ),
    pytest.param(
        betterlite_policy.PolicyConfig, litellm_policy.PolicyConfig, {},
        id="PolicyConfig-defaults",
    ),
    pytest.param(
        betterlite_policy.PolicyConfig, litellm_policy.PolicyConfig,
        {"policies": {"global-baseline": {"guardrails": {"add": ["pii_blocker"]}}}},
        id="PolicyConfig-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyMatchContext, litellm_resolver.PolicyMatchContext, {},
        id="PolicyMatchContext-defaults",
    ),
    pytest.param(
        betterlite_resolver.PolicyMatchContext, litellm_resolver.PolicyMatchContext,
        {"team_alias": "healthcare-team", "key_alias": "dev-key", "model": "gpt-4",
         "tags": ["healthcare"]},
        id="PolicyMatchContext-values",
    ),
    pytest.param(
        betterlite_resolver.ResolvedPolicy, litellm_resolver.ResolvedPolicy,
        {"policy_name": "healthcare-compliance"}, id="ResolvedPolicy-minimal",
    ),
    pytest.param(
        betterlite_resolver.ResolvedPolicy, litellm_resolver.ResolvedPolicy,
        {"policy_name": "healthcare-compliance", "guardrails": ["pii_blocker"],
         "inheritance_chain": ["global-baseline", "healthcare-compliance"]},
        id="ResolvedPolicy-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyScopeResponse, litellm_resolver.PolicyScopeResponse, {},
        id="PolicyScopeResponse-defaults",
    ),
    pytest.param(
        betterlite_resolver.PolicyScopeResponse, litellm_resolver.PolicyScopeResponse,
        {"teams": ["healthcare-team"], "keys": [], "models": ["gpt-4"], "tags": ["prod"]},
        id="PolicyScopeResponse-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyGuardrailsResponse,
        litellm_resolver.PolicyGuardrailsResponse, {},
        id="PolicyGuardrailsResponse-defaults",
    ),
    pytest.param(
        betterlite_resolver.PolicyGuardrailsResponse,
        litellm_resolver.PolicyGuardrailsResponse,
        {"add": ["pii_blocker"], "remove": ["phi_blocker"]},
        id="PolicyGuardrailsResponse-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyInfoResponse, litellm_resolver.PolicyInfoResponse,
        {"policy_name": "global-baseline", "scope": {}, "guardrails": {},
         "resolved_guardrails": ["pii_blocker"], "inheritance_chain": ["global-baseline"]},
        id="PolicyInfoResponse-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyInfoResponse, litellm_resolver.PolicyInfoResponse,
        {"policy_name": "healthcare-compliance", "inherit": "global-baseline",
         "scope": {"teams": ["healthcare-team"]}, "guardrails": {"add": ["hipaa_audit"]},
         "resolved_guardrails": ["pii_blocker", "hipaa_audit"],
         "inheritance_chain": ["global-baseline", "healthcare-compliance"]},
        id="PolicyInfoResponse-values",
    ),
    pytest.param(
        betterlite_resolver.PolicySummaryItem, litellm_resolver.PolicySummaryItem,
        {"scope": {}, "guardrails": {}, "resolved_guardrails": [],
         "inheritance_chain": []},
        id="PolicySummaryItem-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicySummaryItem, litellm_resolver.PolicySummaryItem,
        {"inherit": "global-baseline", "scope": {"models": ["gpt-4"]},
         "guardrails": {"remove": ["pii_blocker"]}, "resolved_guardrails": [],
         "inheritance_chain": ["global-baseline"]},
        id="PolicySummaryItem-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyListResponse, litellm_resolver.PolicyListResponse,
        {"policies": {"global-baseline": {"scope": {}, "guardrails": {},
                                          "resolved_guardrails": [],
                                          "inheritance_chain": ["global-baseline"]}},
         "total_count": 1},
        id="PolicyListResponse-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyTestResponse, litellm_resolver.PolicyTestResponse,
        {"context": {}, "matching_policies": ["global-baseline"],
         "resolved_guardrails": ["pii_blocker"]},
        id="PolicyTestResponse-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyTestResponse, litellm_resolver.PolicyTestResponse,
        {"context": {"team_alias": "t", "model": "gpt-4"},
         "matching_policies": ["global-baseline"], "resolved_guardrails": [],
         "message": "matched"},
        id="PolicyTestResponse-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyConditionRequest, litellm_resolver.PolicyConditionRequest,
        {}, id="PolicyConditionRequest-defaults",
    ),
    pytest.param(
        betterlite_resolver.PolicyConditionRequest, litellm_resolver.PolicyConditionRequest,
        {"model": "gpt-4.*"}, id="PolicyConditionRequest-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyCreateRequest, resolver_models.PolicyCreateRequest,
        {"policy_name": "dev-safety"}, id="PolicyCreateRequest-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyCreateRequest, resolver_models.PolicyCreateRequest,
        {"policy_name": "dev-safety", "inherit": "global-baseline", "description": "dev",
         "guardrails_add": ["toxicity_filter"], "guardrails_remove": ["phi_blocker"],
         "condition": {"model": "gpt-4"}, "pipeline": {"mode": "post_call", "steps": []}},
        id="PolicyCreateRequest-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyUpdateRequest, resolver_models.PolicyUpdateRequest, {},
        id="PolicyUpdateRequest-defaults",
    ),
    pytest.param(
        betterlite_resolver.PolicyUpdateRequest, resolver_models.PolicyUpdateRequest,
        {"policy_name": "renamed", "inherit": "global-baseline", "description": "updated",
         "guardrails_add": ["toxicity_filter"], "guardrails_remove": ["phi_blocker"],
         "condition": {"model": "gpt-4"}, "pipeline": {"mode": "pre_call"}},
        id="PolicyUpdateRequest-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyDBResponse, resolver_models.PolicyDBResponse,
        {"policy_id": "policy-1", "policy_name": "global-baseline"},
        id="PolicyDBResponse-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyDBResponse, resolver_models.PolicyDBResponse,
        {
            "policy_id": "policy-2",
            "policy_name": "healthcare-compliance",
            "version_number": 2,
            "version_status": "draft",
            "parent_version_id": "policy-1",
            "is_latest": False,
            "published_at": "2024-01-02T03:04:05+00:00",
            "production_at": "2024-01-03T00:00:00Z",
            "inherit": "global-baseline",
            "description": "HIPAA compliance",
            "guardrails_add": ["hipaa_audit"],
            "guardrails_remove": ["phi_blocker"],
            "condition": {"model": "gpt-4"},
            "pipeline": {"mode": "pre_call", "steps": []},
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-04T00:00:00Z",
            "created_by": "alice",
            "updated_by": "bob",
            "definition_location": "config",
        },
        id="PolicyDBResponse-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyListDBResponse, litellm_resolver.PolicyListDBResponse, {},
        id="PolicyListDBResponse-defaults",
    ),
    pytest.param(
        betterlite_resolver.PolicyListDBResponse, litellm_resolver.PolicyListDBResponse,
        {"policies": [{"policy_id": "policy-1", "policy_name": "global-baseline"}],
         "total_count": 1},
        id="PolicyListDBResponse-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyVersionCreateRequest,
        litellm_resolver.PolicyVersionCreateRequest, {},
        id="PolicyVersionCreateRequest-defaults",
    ),
    pytest.param(
        betterlite_resolver.PolicyVersionCreateRequest,
        litellm_resolver.PolicyVersionCreateRequest,
        {"source_policy_id": "policy-1"}, id="PolicyVersionCreateRequest-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyVersionStatusUpdateRequest,
        litellm_resolver.PolicyVersionStatusUpdateRequest,
        {"version_status": "published"}, id="PolicyVersionStatusUpdateRequest-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyVersionListResponse,
        litellm_resolver.PolicyVersionListResponse,
        {"policy_name": "global-baseline"}, id="PolicyVersionListResponse-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyVersionListResponse,
        litellm_resolver.PolicyVersionListResponse,
        {"policy_name": "global-baseline",
         "versions": [{"policy_id": "policy-1", "policy_name": "global-baseline"}],
         "total_count": 1},
        id="PolicyVersionListResponse-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyVersionCompareResponse,
        resolver_models.PolicyVersionCompareResponse,
        {"version_a": {"policy_id": "policy-1", "policy_name": "global-baseline"},
         "version_b": {"policy_id": "policy-2", "policy_name": "global-baseline"}},
        id="PolicyVersionCompareResponse-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyVersionCompareResponse,
        resolver_models.PolicyVersionCompareResponse,
        {"version_a": {"policy_id": "policy-1", "policy_name": "global-baseline"},
         "version_b": {"policy_id": "policy-2", "policy_name": "global-baseline"},
         "field_diffs": {"description": {"version_a": "a", "version_b": "b"}}},
        id="PolicyVersionCompareResponse-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyAttachmentCreateRequest,
        litellm_resolver.PolicyAttachmentCreateRequest,
        {"policy_name": "global-baseline"}, id="PolicyAttachmentCreate-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyAttachmentCreateRequest,
        litellm_resolver.PolicyAttachmentCreateRequest,
        {"policy_name": "dev-safety", "scope": None, "teams": ["healthcare-team"],
         "keys": ["dev-key-*"], "models": ["gpt-4"], "tags": ["health-*"],
         "priority": -5, "default": True},
        id="PolicyAttachmentCreate-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyAttachmentDBResponse,
        litellm_resolver.PolicyAttachmentDBResponse,
        {"attachment_id": "attachment-1", "policy_name": "global-baseline"},
        id="PolicyAttachmentDB-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyAttachmentDBResponse,
        litellm_resolver.PolicyAttachmentDBResponse,
        {"attachment_id": "attachment-2", "policy_name": "dev-safety", "scope": "*",
         "teams": ["healthcare-team"], "keys": [], "models": ["gpt-4"], "tags": ["prod"],
         "priority": 7, "default": False, "created_at": "2024-01-01T00:00:00Z",
         "updated_at": "2024-01-02T00:00:00Z", "created_by": "alice", "updated_by": "bob",
         "definition_location": "config"},
        id="PolicyAttachmentDB-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyAttachmentListResponse,
        litellm_resolver.PolicyAttachmentListResponse, {},
        id="PolicyAttachmentList-defaults",
    ),
    pytest.param(
        betterlite_resolver.PolicyAttachmentListResponse,
        litellm_resolver.PolicyAttachmentListResponse,
        {"attachments": [{"attachment_id": "attachment-1",
                          "policy_name": "global-baseline"}], "total_count": 1},
        id="PolicyAttachmentList-values",
    ),
    pytest.param(
        betterlite_resolver.PipelineTestRequest, resolver_models.PipelineTestRequest,
        {"pipeline": {"mode": "pre_call", "steps": []},
         "test_messages": [{"role": "user", "content": "hello"}]},
        id="PipelineTestRequest-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyResolveRequest, litellm_resolver.PolicyResolveRequest, {},
        id="PolicyResolveRequest-defaults",
    ),
    pytest.param(
        betterlite_resolver.PolicyResolveRequest, litellm_resolver.PolicyResolveRequest,
        {"team_alias": "healthcare-team", "key_alias": "dev-key", "model": "gpt-4",
         "tags": ["healthcare"]},
        id="PolicyResolveRequest-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyMatchDetail, litellm_resolver.PolicyMatchDetail,
        {"policy_name": "global-baseline", "matched_via": "scope:*"},
        id="PolicyMatchDetail-minimal",
    ),
    pytest.param(
        betterlite_resolver.PolicyMatchDetail, litellm_resolver.PolicyMatchDetail,
        {"policy_name": "healthcare-compliance", "matched_via": "tag:healthcare",
         "guardrails_added": ["hipaa_audit"]},
        id="PolicyMatchDetail-values",
    ),
    pytest.param(
        betterlite_resolver.PolicyResolveResponse, litellm_resolver.PolicyResolveResponse, {},
        id="PolicyResolveResponse-defaults",
    ),
    pytest.param(
        betterlite_resolver.PolicyResolveResponse, litellm_resolver.PolicyResolveResponse,
        {"effective_guardrails": ["pii_blocker"],
         "matched_policies": [{"policy_name": "global-baseline",
                               "matched_via": "scope:*"}]},
        id="PolicyResolveResponse-values",
    ),
    pytest.param(
        betterlite_resolver.AttachmentImpactResponse,
        litellm_resolver.AttachmentImpactResponse, {},
        id="AttachmentImpactResponse-defaults",
    ),
    pytest.param(
        betterlite_resolver.AttachmentImpactResponse,
        litellm_resolver.AttachmentImpactResponse,
        {"affected_keys_count": 4, "affected_teams_count": 2, "unnamed_keys_count": 1,
         "unnamed_teams_count": 1, "sample_keys": ["dev-key"], "sample_teams": ["health"]},
        id="AttachmentImpactResponse-values",
    ),
    pytest.param(
        betterlite_pipeline.PipelineStep, litellm_pipeline.PipelineStep,
        {"guardrail": "pii_blocker"}, id="PipelineStep-defaults",
    ),
    pytest.param(
        betterlite_pipeline.PipelineStep, litellm_pipeline.PipelineStep,
        {"guardrail": "pii_blocker", "on_fail": "modify_response", "on_pass": "next",
         "on_error": "allow", "pass_data": True, "modify_response_message": "blocked"},
        id="PipelineStep-values",
    ),
    pytest.param(
        betterlite_pipeline.GuardrailPipeline, litellm_pipeline.GuardrailPipeline,
        {"mode": "pre_call", "steps": [{"guardrail": "pii_blocker"}]},
        id="GuardrailPipeline-minimal",
    ),
    pytest.param(
        betterlite_pipeline.GuardrailPipeline, litellm_pipeline.GuardrailPipeline,
        {"mode": "post_call",
         "steps": [{"guardrail": "pii_blocker", "on_fail": "next"},
                   {"guardrail": "toxicity_filter", "on_fail": "block",
                    "on_pass": "allow"}]},
        id="GuardrailPipeline-values",
    ),
    pytest.param(
        betterlite_pipeline.PipelineStepResult, pipeline_models.PipelineStepResult,
        {"guardrail_name": "pii_blocker", "outcome": "fail", "action_taken": "block"},
        id="PipelineStepResult-minimal",
    ),
    pytest.param(
        betterlite_pipeline.PipelineStepResult, pipeline_models.PipelineStepResult,
        {"guardrail_name": "pii_blocker", "outcome": "error", "action_taken": "next",
         "modified_data": {"messages": []}, "error_detail": "timeout",
         "duration_seconds": 0.05},
        id="PipelineStepResult-values",
    ),
    pytest.param(
        betterlite_pipeline.PipelineStepResult, pipeline_models.PipelineStepResult,
        {"guardrail_name": "pii_blocker", "outcome": "pass", "action_taken": "allow"},
        id="PipelineStepResult-pass",
    ),
    pytest.param(
        betterlite_pipeline.PipelineExecutionResult,
        pipeline_models.PipelineExecutionResult,
        {"terminal_action": "block", "step_results": []},
        id="PipelineExecutionResult-minimal",
    ),
    pytest.param(
        betterlite_pipeline.PipelineExecutionResult,
        pipeline_models.PipelineExecutionResult,
        {"terminal_action": "modify_response",
         "step_results": [{"guardrail_name": "pii_blocker", "outcome": "fail",
                           "action_taken": "modify_response"}],
         "modified_data": {"messages": []}, "error_message": "blocked",
         "modify_response_message": "the request was blocked"},
        id="PipelineExecutionResult-values",
    ),
    pytest.param(
        betterlite_validation.PolicyValidationError,
        litellm_validation.PolicyValidationError,
        {"policy_name": "global-baseline", "error_type": "invalid_guardrail",
         "message": "unknown guardrail"},
        id="PolicyValidationError-minimal",
    ),
    pytest.param(
        betterlite_validation.PolicyValidationError,
        litellm_validation.PolicyValidationError,
        {"policy_name": "global-baseline", "error_type": "circular_inheritance",
         "message": "policy inherits itself", "field": "inherit", "value": "global-baseline"},
        id="PolicyValidationError-values",
    ),
    pytest.param(
        betterlite_validation.PolicyValidationResponse,
        litellm_validation.PolicyValidationResponse,
        {"valid": True}, id="PolicyValidationResponse-minimal",
    ),
    pytest.param(
        betterlite_validation.PolicyValidationResponse,
        litellm_validation.PolicyValidationResponse,
        {"valid": False,
         "errors": [{"policy_name": "global-baseline", "error_type": "invalid_scope",
                     "message": "bad scope"}],
         "warnings": [{"policy_name": "global-baseline", "error_type": "invalid_team",
                       "message": "unknown team"}]},
        id="PolicyValidationResponse-values",
    ),
    pytest.param(
        betterlite_validation.PolicyValidateRequest,
        validation_models.PolicyValidateRequest,
        {"policies": {"global-baseline": {"guardrails": {"add": ["pii_blocker"]}}}},
        id="PolicyValidateRequest-minimal",
    ),
]

REQUIRED_PARAMS = [
    pytest.param(betterlite_policy.PolicyAttachment, litellm_policy.PolicyAttachment,
                 id="PolicyAttachment"),
    pytest.param(betterlite_resolver.ResolvedPolicy, litellm_resolver.ResolvedPolicy,
                 id="ResolvedPolicy"),
    pytest.param(betterlite_resolver.PolicyInfoResponse, litellm_resolver.PolicyInfoResponse,
                 id="PolicyInfoResponse"),
    pytest.param(betterlite_resolver.PolicySummaryItem, litellm_resolver.PolicySummaryItem,
                 id="PolicySummaryItem"),
    pytest.param(betterlite_resolver.PolicyListResponse, litellm_resolver.PolicyListResponse,
                 id="PolicyListResponse"),
    pytest.param(betterlite_resolver.PolicyTestResponse, litellm_resolver.PolicyTestResponse,
                 id="PolicyTestResponse"),
    pytest.param(betterlite_resolver.PolicyCreateRequest,
                 resolver_models.PolicyCreateRequest, id="PolicyCreateRequest"),
    pytest.param(betterlite_resolver.PolicyDBResponse, resolver_models.PolicyDBResponse,
                 id="PolicyDBResponse"),
    pytest.param(betterlite_resolver.PolicyVersionStatusUpdateRequest,
                 litellm_resolver.PolicyVersionStatusUpdateRequest,
                 id="PolicyVersionStatusUpdateRequest"),
    pytest.param(betterlite_resolver.PolicyVersionListResponse,
                 litellm_resolver.PolicyVersionListResponse, id="PolicyVersionListResponse"),
    pytest.param(betterlite_resolver.PolicyVersionCompareResponse,
                 resolver_models.PolicyVersionCompareResponse,
                 id="PolicyVersionCompareResponse"),
    pytest.param(betterlite_resolver.PolicyAttachmentCreateRequest,
                 litellm_resolver.PolicyAttachmentCreateRequest,
                 id="PolicyAttachmentCreateRequest"),
    pytest.param(betterlite_resolver.PolicyAttachmentDBResponse,
                 litellm_resolver.PolicyAttachmentDBResponse,
                 id="PolicyAttachmentDBResponse"),
    pytest.param(betterlite_resolver.PipelineTestRequest,
                 resolver_models.PipelineTestRequest, id="PipelineTestRequest"),
    pytest.param(betterlite_resolver.PolicyMatchDetail, litellm_resolver.PolicyMatchDetail,
                 id="PolicyMatchDetail"),
    pytest.param(betterlite_pipeline.PipelineStep, litellm_pipeline.PipelineStep,
                 id="PipelineStep"),
    pytest.param(betterlite_pipeline.GuardrailPipeline, litellm_pipeline.GuardrailPipeline,
                 id="GuardrailPipeline"),
    pytest.param(betterlite_pipeline.PipelineStepResult, pipeline_models.PipelineStepResult,
                 id="PipelineStepResult"),
    pytest.param(betterlite_pipeline.PipelineExecutionResult,
                 pipeline_models.PipelineExecutionResult, id="PipelineExecutionResult"),
    pytest.param(betterlite_validation.PolicyValidationError,
                 litellm_validation.PolicyValidationError, id="PolicyValidationError"),
    pytest.param(betterlite_validation.PolicyValidationResponse,
                 litellm_validation.PolicyValidationResponse,
                 id="PolicyValidationResponse"),
    pytest.param(betterlite_validation.PolicyValidateRequest,
                 validation_models.PolicyValidateRequest, id="PolicyValidateRequest"),
]

POLICY_CONFIG: dict[str, JsonValue] = {
    "policies": {
        "global-baseline": {
            "description": "Base guardrails for all requests",
            "guardrails": {"add": ["pii_blocker", "phi_blocker"]},
        },
        "healthcare-compliance": {
            "inherit": "global-baseline",
            "description": "HIPAA compliance for healthcare",
            "guardrails": {"add": ["hipaa_audit"], "remove": ["phi_blocker"]},
            "condition": {"model": ["gpt-4", "gpt-4-turbo"]},
            "pipeline": {
                "mode": "pre_call",
                "steps": [
                    {"guardrail": "hipaa_audit", "on_fail": "block", "on_pass": "next"},
                    {"guardrail": "toxicity_filter", "on_fail": "modify_response",
                     "on_error": "allow", "pass_data": True,
                     "modify_response_message": "the request was blocked"},
                ],
            },
        },
    },
}

EMPTY_PAYLOAD: dict[str, JsonValue] = {}


@pytest.mark.parametrize(("betterlite_model", "reference_model", "payload"), MODEL_PARAMS)
def test_model_names_match_litellm(
    betterlite_model: type[BaseModel],
    reference_model: type[BaseModel],
    payload: dict[str, JsonValue],
) -> None:
    reference = reference_model.model_validate(payload)
    ours = betterlite_model.model_validate(payload)
    assert ours.model_dump_json() == reference.model_dump_json(), betterlite_model.__name__


@pytest.mark.parametrize(("betterlite_model", "reference_model"), REQUIRED_PARAMS)
def test_required_names_match(
    betterlite_model: type[BaseModel], reference_model: type[BaseModel],
) -> None:
    ours = {name for name, field in betterlite_model.model_fields.items() if field.is_required()}
    reference = {
        name for name, field in reference_model.model_fields.items() if field.is_required()
    }
    assert ours == reference, betterlite_model.__name__
    with pytest.raises(ValidationError):
        _ = reference_model.model_validate(EMPTY_PAYLOAD)
    with pytest.raises(ValidationError):
        _ = betterlite_model.model_validate(EMPTY_PAYLOAD)


@pytest.mark.parametrize(("betterlite_model", "reference_model", "payload"), MODEL_PARAMS)
def test_extra_names_match_litellm(
    betterlite_model: type[BaseModel],
    reference_model: type[BaseModel],
    payload: dict[str, JsonValue],
) -> None:
    with_extra: dict[str, JsonValue] = dict(payload)
    with_extra["unknown_field"] = "x"
    reference: BaseModel | None = None
    ours: BaseModel | None = None
    try:
        reference = reference_model.model_validate(with_extra)
    except ValidationError:
        reference = None
    try:
        ours = betterlite_model.model_validate(with_extra)
    except ValidationError:
        ours = None
    assert (reference is None) == (ours is None), betterlite_model.__name__
    if reference is None:
        return
    assert ours is not None
    assert ours.model_dump_json() == reference.model_dump_json(), betterlite_model.__name__


def test_names_policy_types_match() -> None:
    pairs: list[tuple[type[BaseModel], type[BaseModel]]] = [
        (betterlite_policy.PolicyCondition, litellm_policy.PolicyCondition),
        (betterlite_policy.PolicyScope, litellm_policy.PolicyScope),
        (betterlite_policy.PolicyGuardrails, litellm_policy.PolicyGuardrails),
        (betterlite_policy.Policy, litellm_policy.Policy),
        (betterlite_policy.PolicyAttachment, litellm_policy.PolicyAttachment),
        (betterlite_policy.PolicyConfig, litellm_policy.PolicyConfig),
    ]
    for ours, reference in pairs:
        assert ours.__name__ == reference.__name__
    _ = (
        betterlite_policy.PolicyScope.get_teams,
        betterlite_policy.PolicyScope.get_keys,
        betterlite_policy.PolicyScope.get_models,
        betterlite_policy.PolicyScope.get_tags,
        betterlite_policy.PolicyGuardrails.get_add,
        betterlite_policy.PolicyGuardrails.get_remove,
        betterlite_policy.PolicyAttachment.is_global,
        betterlite_policy.PolicyAttachment.to_policy_scope,
        litellm_policy.PolicyScope.get_teams,
        litellm_policy.PolicyScope.get_keys,
        litellm_policy.PolicyScope.get_models,
        litellm_policy.PolicyScope.get_tags,
        litellm_policy.PolicyGuardrails.get_add,
        litellm_policy.PolicyGuardrails.get_remove,
        litellm_policy.PolicyAttachment.is_global,
        litellm_policy.PolicyAttachment.to_policy_scope,
    )


def test_names_resolver_match() -> None:
    pairs: list[tuple[type[BaseModel], type[BaseModel]]] = [
        (betterlite_resolver.PolicyMatchContext, litellm_resolver.PolicyMatchContext),
        (betterlite_resolver.ResolvedPolicy, litellm_resolver.ResolvedPolicy),
        (betterlite_resolver.PolicyScopeResponse, litellm_resolver.PolicyScopeResponse),
        (
            betterlite_resolver.PolicyGuardrailsResponse,
            litellm_resolver.PolicyGuardrailsResponse,
        ),
        (betterlite_resolver.PolicyInfoResponse, litellm_resolver.PolicyInfoResponse),
        (betterlite_resolver.PolicySummaryItem, litellm_resolver.PolicySummaryItem),
        (betterlite_resolver.PolicyListResponse, litellm_resolver.PolicyListResponse),
        (betterlite_resolver.PolicyTestResponse, litellm_resolver.PolicyTestResponse),
        (
            betterlite_resolver.PolicyConditionRequest,
            litellm_resolver.PolicyConditionRequest,
        ),
        (betterlite_resolver.PolicyCreateRequest, resolver_models.PolicyCreateRequest),
        (betterlite_resolver.PolicyUpdateRequest, resolver_models.PolicyUpdateRequest),
        (betterlite_resolver.PolicyDBResponse, resolver_models.PolicyDBResponse),
        (
            betterlite_resolver.PolicyListDBResponse,
            litellm_resolver.PolicyListDBResponse,
        ),
        (
            betterlite_resolver.PolicyVersionCreateRequest,
            litellm_resolver.PolicyVersionCreateRequest,
        ),
        (
            betterlite_resolver.PolicyVersionStatusUpdateRequest,
            litellm_resolver.PolicyVersionStatusUpdateRequest,
        ),
        (
            betterlite_resolver.PolicyVersionListResponse,
            litellm_resolver.PolicyVersionListResponse,
        ),
        (
            betterlite_resolver.PolicyVersionCompareResponse,
            resolver_models.PolicyVersionCompareResponse,
        ),
        (
            betterlite_resolver.PolicyAttachmentCreateRequest,
            litellm_resolver.PolicyAttachmentCreateRequest,
        ),
        (
            betterlite_resolver.PolicyAttachmentDBResponse,
            litellm_resolver.PolicyAttachmentDBResponse,
        ),
        (
            betterlite_resolver.PolicyAttachmentListResponse,
            litellm_resolver.PolicyAttachmentListResponse,
        ),
        (betterlite_resolver.PipelineTestRequest, resolver_models.PipelineTestRequest),
        (
            betterlite_resolver.PolicyResolveRequest,
            litellm_resolver.PolicyResolveRequest,
        ),
        (betterlite_resolver.PolicyMatchDetail, litellm_resolver.PolicyMatchDetail),
        (
            betterlite_resolver.PolicyResolveResponse,
            litellm_resolver.PolicyResolveResponse,
        ),
        (
            betterlite_resolver.AttachmentImpactResponse,
            litellm_resolver.AttachmentImpactResponse,
        ),
    ]
    for ours, reference in pairs:
        assert ours.__name__ == reference.__name__


def test_names_pipeline_match() -> None:
    assert (
        betterlite_pipeline.VALID_PIPELINE_ACTIONS,
        betterlite_pipeline.VALID_PIPELINE_MODES,
    ) == (
        litellm_pipeline.VALID_PIPELINE_ACTIONS,
        litellm_pipeline.VALID_PIPELINE_MODES,
    )
    pairs: list[tuple[type[BaseModel], type[BaseModel]]] = [
        (betterlite_pipeline.PipelineStep, litellm_pipeline.PipelineStep),
        (betterlite_pipeline.GuardrailPipeline, litellm_pipeline.GuardrailPipeline),
        (betterlite_pipeline.PipelineStepResult, pipeline_models.PipelineStepResult),
        (
            betterlite_pipeline.PipelineExecutionResult,
            pipeline_models.PipelineExecutionResult,
        ),
    ]
    for ours, reference in pairs:
        assert ours.__name__ == reference.__name__
    _ = (
        betterlite_pipeline.PipelineStep.validate_action,
        betterlite_pipeline.GuardrailPipeline.validate_mode,
        litellm_pipeline.PipelineStep.validate_action,
        litellm_pipeline.GuardrailPipeline.validate_mode,
    )


def test_names_validation_match() -> None:
    assert (
        betterlite_validation.PolicyValidationErrorType.__name__
        == litellm_validation.PolicyValidationErrorType.__name__
    )
    pairs: list[tuple[type[BaseModel], type[BaseModel]]] = [
        (
            betterlite_validation.PolicyValidationError,
            litellm_validation.PolicyValidationError,
        ),
        (
            betterlite_validation.PolicyValidationResponse,
            litellm_validation.PolicyValidationResponse,
        ),
        (
            betterlite_validation.PolicyValidateRequest,
            validation_models.PolicyValidateRequest,
        ),
    ]
    for ours, reference in pairs:
        assert ours.__name__ == reference.__name__


def test_names_reexports_match() -> None:
    assert betterlite_compat.__all__ == litellm_compat.__all__
    pairs = [
        (betterlite_compat.Policy, betterlite_policy.Policy),
        (betterlite_compat.PolicyConfig, betterlite_policy.PolicyConfig),
        (betterlite_compat.PolicyGuardrails, betterlite_policy.PolicyGuardrails),
        (betterlite_compat.PolicyMatchContext, betterlite_resolver.PolicyMatchContext),
        (betterlite_compat.PolicyScope, betterlite_policy.PolicyScope),
        (
            betterlite_compat.PolicyValidateRequest,
            betterlite_validation.PolicyValidateRequest,
        ),
        (
            betterlite_compat.PolicyValidationError,
            betterlite_validation.PolicyValidationError,
        ),
        (
            betterlite_compat.PolicyValidationErrorType,
            betterlite_validation.PolicyValidationErrorType,
        ),
        (
            betterlite_compat.PolicyValidationResponse,
            betterlite_validation.PolicyValidationResponse,
        ),
        (betterlite_compat.ResolvedPolicy, betterlite_resolver.ResolvedPolicy),
    ]
    for exported, defined in pairs:
        assert exported is defined


def test_names_mapping_access() -> None:
    policy = betterlite_policy.Policy(inherit="global-baseline")
    assert "inherit" in policy
    assert policy.get("inherit") == "global-baseline"
    assert policy["inherit"] == "global-baseline"


def test_scope_names_default() -> None:
    reference = litellm_policy.PolicyScope()
    ours = betterlite_policy.PolicyScope()
    assert ours.get_teams() == reference.get_teams() == ["*"]
    assert ours.get_keys() == reference.get_keys() == ["*"]
    assert ours.get_models() == reference.get_models() == ["*"]
    assert ours.get_tags() == reference.get_tags() == []
    scoped_reference = litellm_policy.PolicyScope(
        teams=[], keys=["key"], models=["gpt-4"], tags=[],
    )
    scoped_ours = betterlite_policy.PolicyScope(
        teams=[], keys=["key"], models=["gpt-4"], tags=[],
    )
    assert scoped_ours.get_teams() == scoped_reference.get_teams() == ["*"]
    assert scoped_ours.get_keys() == scoped_reference.get_keys() == ["key"]
    assert scoped_ours.get_models() == scoped_reference.get_models() == ["gpt-4"]
    assert scoped_ours.get_tags() == scoped_reference.get_tags() == []


def test_guardrails_names_default() -> None:
    reference = litellm_policy.PolicyGuardrails()
    ours = betterlite_policy.PolicyGuardrails()
    assert ours.get_add() == reference.get_add() == []
    assert ours.get_remove() == reference.get_remove() == []
    empty_reference = litellm_policy.PolicyGuardrails(add=[], remove=["pii_blocker"])
    empty_ours = betterlite_policy.PolicyGuardrails(add=[], remove=["pii_blocker"])
    assert empty_ours.get_add() == empty_reference.get_add() == []
    assert empty_ours.get_remove() == empty_reference.get_remove() == ["pii_blocker"]


def test_attachment_names_scope() -> None:
    global_reference = litellm_policy.PolicyAttachment(
        policy="global-baseline", scope="*",
        teams=["healthcare-team"], keys=["dev-key-*"], models=["gpt-4"], tags=["prod"],
    )
    global_ours = betterlite_policy.PolicyAttachment(
        policy="global-baseline", scope="*",
        teams=["healthcare-team"], keys=["dev-key-*"], models=["gpt-4"], tags=["prod"],
    )
    assert global_ours.is_global() == global_reference.is_global()
    assert global_ours.is_global()
    global_ours_scope = global_ours.to_policy_scope()
    global_reference_scope = global_reference.to_policy_scope()
    assert global_ours_scope.model_dump_json() == global_reference_scope.model_dump_json()
    scoped_reference = litellm_policy.PolicyAttachment(
        policy="dev-safety", teams=["healthcare-team"], keys=["dev-key-*"], tags=["prod"],
    )
    scoped_ours = betterlite_policy.PolicyAttachment(
        policy="dev-safety", teams=["healthcare-team"], keys=["dev-key-*"], tags=["prod"],
    )
    assert scoped_ours.is_global() == scoped_reference.is_global()
    scoped_ours_scope = scoped_ours.to_policy_scope()
    scoped_reference_scope = scoped_reference.to_policy_scope()
    assert scoped_ours_scope.model_dump_json() == scoped_reference_scope.model_dump_json()


@pytest.mark.parametrize("priority", [-2147483648, 2147483647])
def test_priority_names_bounds(priority: int) -> None:
    reference = litellm_policy.PolicyAttachment(policy="p", priority=priority)
    ours = betterlite_policy.PolicyAttachment(policy="p", priority=priority)
    assert ours.model_dump_json() == reference.model_dump_json()


@pytest.mark.parametrize("priority", [-2147483649, 2147483648])
def test_priority_names_rejected(priority: int) -> None:
    attachment: dict[str, JsonValue] = {"policy": "p", "priority": priority}
    create: dict[str, JsonValue] = {"policy_name": "p", "priority": priority}
    with pytest.raises(ValidationError):
        _ = litellm_policy.PolicyAttachment.model_validate(attachment)
    with pytest.raises(ValidationError):
        _ = betterlite_policy.PolicyAttachment.model_validate(attachment)
    with pytest.raises(ValidationError):
        _ = litellm_resolver.PolicyAttachmentCreateRequest.model_validate(create)
    with pytest.raises(ValidationError):
        _ = betterlite_resolver.PolicyAttachmentCreateRequest.model_validate(create)


@pytest.mark.parametrize("action", ["allow", "block", "next", "modify_response"])
def test_pipeline_names_actions(action: str) -> None:
    payload: dict[str, JsonValue] = {
        "guardrail": "g", "on_fail": action, "on_pass": action, "on_error": action,
    }
    reference = litellm_pipeline.PipelineStep.model_validate(payload)
    ours = betterlite_pipeline.PipelineStep.model_validate(payload)
    assert ours.model_dump_json() == reference.model_dump_json()


@pytest.mark.parametrize("action", ["", "skip", "ALLOW"])
def test_pipeline_names_reject(action: str) -> None:
    payload: dict[str, JsonValue] = {"guardrail": "g", "on_fail": action}
    with pytest.raises(ValidationError):
        _ = litellm_pipeline.PipelineStep.model_validate(payload)
    with pytest.raises(ValidationError):
        _ = betterlite_pipeline.PipelineStep.model_validate(payload)


@pytest.mark.parametrize("mode", ["pre_call", "post_call"])
def test_pipeline_names_modes(mode: str) -> None:
    payload: dict[str, JsonValue] = {"mode": mode, "steps": [{"guardrail": "g"}]}
    reference = litellm_pipeline.GuardrailPipeline.model_validate(payload)
    ours = betterlite_pipeline.GuardrailPipeline.model_validate(payload)
    assert ours.model_dump_json() == reference.model_dump_json()


@pytest.mark.parametrize("payload", [
    {"mode": "during_call", "steps": [{"guardrail": "g"}]},
    {"mode": "pre_call", "steps": []},
    {"mode": "pre_call"},
])
def test_pipeline_names_bad_shapes(payload: dict[str, JsonValue]) -> None:
    with pytest.raises(ValidationError):
        _ = litellm_pipeline.GuardrailPipeline.model_validate(payload)
    with pytest.raises(ValidationError):
        _ = betterlite_pipeline.GuardrailPipeline.model_validate(payload)


def test_enum_names_match() -> None:
    ours_members = list(betterlite_validation.PolicyValidationErrorType)
    reference_members = list(litellm_validation.PolicyValidationErrorType)
    assert [member.name for member in ours_members] == [
        member.name for member in reference_members
    ]
    for ours_member, reference_member in zip(ours_members, reference_members, strict=True):
        ours = betterlite_validation.PolicyValidationError(
            policy_name="p", error_type=ours_member, message="m",
        )
        reference = litellm_validation.PolicyValidationError(
            policy_name="p", error_type=reference_member, message="m",
        )
        assert ours.model_dump_json() == reference.model_dump_json()


def test_literal_names_rejected() -> None:
    payload: dict[str, JsonValue] = {
        "guardrail_name": "g", "outcome": "other", "action_taken": "allow",
    }
    with pytest.raises(ValidationError):
        _ = pipeline_models.PipelineStepResult.model_validate(payload)
    with pytest.raises(ValidationError):
        _ = betterlite_pipeline.PipelineStepResult.model_validate(payload)


def test_definition_names_rejected() -> None:
    payload: dict[str, JsonValue] = {
        "policy_id": "id", "policy_name": "p", "definition_location": "memory",
    }
    with pytest.raises(ValidationError):
        _ = resolver_models.PolicyDBResponse.model_validate(payload)
    with pytest.raises(ValidationError):
        _ = betterlite_resolver.PolicyDBResponse.model_validate(payload)
    attachment: dict[str, JsonValue] = {
        "attachment_id": "a", "policy_name": "p", "definition_location": "memory",
    }
    with pytest.raises(ValidationError):
        _ = litellm_resolver.PolicyAttachmentDBResponse.model_validate(attachment)
    with pytest.raises(ValidationError):
        _ = betterlite_resolver.PolicyAttachmentDBResponse.model_validate(attachment)


def test_policy_names_dump_match() -> None:
    reference = litellm_policy.PolicyConfig.model_validate(POLICY_CONFIG)
    ours = betterlite_policy.PolicyConfig.model_validate(POLICY_CONFIG)
    assert ours.model_dump_json() == reference.model_dump_json()


def test_execution_names_exclude() -> None:
    reference = pipeline_models.PipelineExecutionResult(
        terminal_action="block", step_results=[],
    )
    ours = betterlite_pipeline.PipelineExecutionResult(
        terminal_action="block", step_results=[],
    )
    failure = ValueError("guardrail call failed")
    reference.original_exception = failure
    ours.original_exception = failure
    assert ours.original_exception is failure
    assert reference.original_exception is failure
    assert ours.model_dump_json() == reference.model_dump_json()
