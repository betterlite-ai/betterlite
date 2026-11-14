import litellm.types.proxy.agent_identity as litellm_agent_identity
import litellm.types.proxy.auth.auth_checks as litellm_auth_checks
import litellm.types.proxy.callback_logs_endpoints as litellm_callback_logs
import litellm.types.proxy.carried_budget_state as litellm_carried_budget
import litellm.types.proxy.claude_code_endpoints as litellm_claude_code
import litellm.types.proxy.cloudzero_endpoints as litellm_cloudzero
import litellm.types.proxy.compliance_endpoints as litellm_compliance
import litellm.types.proxy.control_plane_endpoints as litellm_control_plane
import litellm.types.proxy.discovery_endpoints.agent_skills_endpoints as litellm_agent_skills
import litellm.types.proxy.discovery_endpoints.ui_discovery_endpoints as litellm_ui_discovery
import litellm.types.proxy.gateway_requests as litellm_gateway
import litellm.types.proxy.litellm_pre_call_utils as litellm_pre_call
import litellm.types.proxy.model_access_group_budget as litellm_access_budget
import litellm.types.proxy.model_deprecation as litellm_deprecation
import litellm.types.proxy.model_listing as litellm_model_listing
import litellm.types.proxy.prompt_endpoints as litellm_prompts
import litellm.types.proxy.spend_capture_rate as litellm_spend_capture
import litellm.types.proxy.vantage_endpoints as litellm_vantage
import pytest
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError

import betterlite.types.proxy.agent_identity as agent_identity
import betterlite.types.proxy.auth.auth_checks as auth_checks
import betterlite.types.proxy.callback_logs_endpoints as callback_logs
import betterlite.types.proxy.carried_budget_state as carried_budget
import betterlite.types.proxy.claude_code_endpoints as claude_code
import betterlite.types.proxy.cloudzero_endpoints as cloudzero
import betterlite.types.proxy.compliance_endpoints as compliance
import betterlite.types.proxy.control_plane_endpoints as control_plane
import betterlite.types.proxy.discovery_endpoints.agent_skills_endpoints as agent_skills
import betterlite.types.proxy.discovery_endpoints.ui_discovery_endpoints as ui_discovery
import betterlite.types.proxy.gateway_requests as gateway_requests
import betterlite.types.proxy.model_access_group_budget as access_budget
import betterlite.types.proxy.model_deprecation as model_deprecation
import betterlite.types.proxy.model_listing as model_listing
import betterlite.types.proxy.prompt_endpoints as prompts
import betterlite.types.proxy.spend_capture_rate as spend_capture
import betterlite.types.proxy.vantage_endpoints as vantage
from betterlite.types.proxy.litellm_pre_call_utils import RedactedDict, SecretFields

MODEL_PAIRS: tuple[tuple[type[BaseModel], type[BaseModel]], ...] = (
    (litellm_agent_identity.EntraIdentityConfig, agent_identity.EntraIdentityConfig),
    (litellm_agent_identity.AgentIdentityBinding, agent_identity.AgentIdentityBinding),
    (litellm_agent_identity.AgentSubject, agent_identity.AgentSubject),
    (litellm_agent_identity.AgentIdentityFailure, agent_identity.AgentIdentityFailure),
    (litellm_agent_identity.ManagedAgentContext, agent_identity.ManagedAgentContext),
    (litellm_agent_identity.VerifiedHumanSubject, agent_identity.VerifiedHumanSubject),
    (
        litellm_agent_identity.MicrosoftInteractiveSubject,
        agent_identity.MicrosoftInteractiveSubject,
    ),
    (litellm_agent_identity.ManagedAgentIdentityStatus, agent_identity.ManagedAgentIdentityStatus),
    (litellm_callback_logs.CallbackLogRecord, callback_logs.CallbackLogRecord),
    (litellm_callback_logs.CallbackLogsRequest, callback_logs.CallbackLogsRequest),
    (litellm_callback_logs.CallbackLogFailure, callback_logs.CallbackLogFailure),
    (litellm_callback_logs.CallbackLogsResponse, callback_logs.CallbackLogsResponse),
    (litellm_carried_budget.KeyBudgetSnapshot, carried_budget.KeyBudgetSnapshot),
    (litellm_carried_budget.TeamBudgetSnapshot, carried_budget.TeamBudgetSnapshot),
    (litellm_carried_budget.UserBudgetSnapshot, carried_budget.UserBudgetSnapshot),
    (litellm_carried_budget.OrgBudgetSnapshot, carried_budget.OrgBudgetSnapshot),
    (litellm_claude_code.PluginAuthor, claude_code.PluginAuthor),
    (litellm_claude_code.PluginOwner, claude_code.PluginOwner),
    (litellm_claude_code.PluginSpec, claude_code.PluginSpec),
    (litellm_claude_code.RegisterPluginRequest, claude_code.RegisterPluginRequest),
    (litellm_claude_code.UpdatePluginRequest, claude_code.UpdatePluginRequest),
    (litellm_claude_code.PluginResponse, claude_code.PluginResponse),
    (litellm_claude_code.RegisterPluginResponse, claude_code.RegisterPluginResponse),
    (litellm_claude_code.PluginListItem, claude_code.PluginListItem),
    (litellm_claude_code.ListPluginsResponse, claude_code.ListPluginsResponse),
    (litellm_claude_code.MarketplacePluginEntry, claude_code.MarketplacePluginEntry),
    (litellm_claude_code.MarketplaceResponse, claude_code.MarketplaceResponse),
    (litellm_cloudzero.CloudZeroInitRequest, cloudzero.CloudZeroInitRequest),
    (litellm_cloudzero.CloudZeroInitResponse, cloudzero.CloudZeroInitResponse),
    (litellm_cloudzero.CloudZeroExportRequest, cloudzero.CloudZeroExportRequest),
    (litellm_cloudzero.CloudZeroExportResponse, cloudzero.CloudZeroExportResponse),
    (litellm_cloudzero.CloudZeroSettingsView, cloudzero.CloudZeroSettingsView),
    (litellm_cloudzero.CloudZeroSettingsUpdate, cloudzero.CloudZeroSettingsUpdate),
    (litellm_compliance.ComplianceCheckResult, compliance.ComplianceCheckResult),
    (litellm_compliance.ComplianceResponse, compliance.ComplianceResponse),
    (litellm_compliance.ComplianceCheckRequest, compliance.ComplianceCheckRequest),
    (litellm_control_plane.WorkerRegistryEntry, control_plane.WorkerRegistryEntry),
    (litellm_agent_skills.AgentSkillsIndexEntry, agent_skills.AgentSkillsIndexEntry),
    (litellm_agent_skills.AgentSkillsIndex, agent_skills.AgentSkillsIndex),
    (litellm_gateway.GatewayRequestBreakdownEntry, gateway_requests.GatewayRequestBreakdownEntry),
    (litellm_gateway.GatewayRequestDailyEntry, gateway_requests.GatewayRequestDailyEntry),
    (
        litellm_gateway.GatewayRequestActivityResponse,
        gateway_requests.GatewayRequestActivityResponse,
    ),
    (litellm_access_budget.ModelAccessGroupBudget, access_budget.ModelAccessGroupBudget),
    (litellm_deprecation.ModelDeprecationInfo, model_deprecation.ModelDeprecationInfo),
    (litellm_deprecation.ModelDeprecationResponse, model_deprecation.ModelDeprecationResponse),
    (litellm_prompts.TestPromptRequest, prompts.TestPromptRequest),
    (litellm_spend_capture.CaptureRateDay, spend_capture.CaptureRateDay),
    (litellm_spend_capture.CaptureRateReport, spend_capture.CaptureRateReport),
    (
        litellm_spend_capture.SpendCaptureRateCheckSettings,
        spend_capture.SpendCaptureRateCheckSettings,
    ),
    (litellm_vantage.VantageInitRequest, vantage.VantageInitRequest),
    (litellm_vantage.VantageInitResponse, vantage.VantageInitResponse),
    (litellm_vantage.VantageExportRequest, vantage.VantageExportRequest),
    (litellm_vantage.VantageDryRunRequest, vantage.VantageDryRunRequest),
    (litellm_vantage.VantageExportResponse, vantage.VantageExportResponse),
    (litellm_vantage.VantageSettingsView, vantage.VantageSettingsView),
    (litellm_vantage.VantageSettingsUpdate, vantage.VantageSettingsUpdate),
)


def strip_volatile(value: JsonValue) -> JsonValue:
    match value:
        case {"$ref": "#/$defs/JsonValue"}:
            return True
        case dict():
            stripped = {
                key: strip_volatile(item)
                for key, item in value.items()
                if key not in {"description", "title", "JsonValue"}
            }
            return without_empty_definitions(stripped)
        case list():
            return [strip_volatile(item) for item in value]
        case bool() | int() | float() | str() | None:
            return value


def without_empty_definitions(schema: dict[str, JsonValue]) -> dict[str, JsonValue]:
    if schema.get("$defs") == {}:
        return {key: item for key, item in schema.items() if key != "$defs"}
    return schema


def test_model_names_match_litellm() -> None:
    for their_model, our_model in MODEL_PAIRS:
        their_schema: dict[str, JsonValue] = their_model.model_json_schema()
        our_schema: dict[str, JsonValue] = our_model.model_json_schema()
        assert strip_volatile(our_schema) == strip_volatile(their_schema), our_model.__name__
        assert our_model.model_config == their_model.model_config, our_model.__name__


def test_typed_dict_names_match() -> None:
    their_secret: dict[str, JsonValue] = TypeAdapter(litellm_pre_call.SecretFields).json_schema()
    our_secret: dict[str, JsonValue] = TypeAdapter(SecretFields).json_schema()
    assert strip_volatile(our_secret) == strip_volatile(their_secret)
    their_metadata: dict[str, JsonValue] = TypeAdapter(
        litellm_model_listing.ModelInfoMetadata
    ).json_schema()
    our_metadata: dict[str, JsonValue] = TypeAdapter(model_listing.ModelInfoMetadata).json_schema()
    assert strip_volatile(our_metadata) == strip_volatile(their_metadata)
    their_model_info: dict[str, JsonValue] = TypeAdapter(
        litellm_model_listing.ModelInfoResponse
    ).json_schema()
    our_model_info: dict[str, JsonValue] = TypeAdapter(
        model_listing.ModelInfoResponse
    ).json_schema()
    assert strip_volatile(our_model_info) == strip_volatile(their_model_info)


def test_literal_names_match() -> None:
    their_mode: dict[str, JsonValue] = TypeAdapter(
        litellm_agent_identity.AgentExecutionMode
    ).json_schema()
    our_mode: dict[str, JsonValue] = TypeAdapter(agent_identity.AgentExecutionMode).json_schema()
    assert strip_volatile(our_mode) == strip_volatile(their_mode)
    their_status: dict[str, JsonValue] = TypeAdapter(
        litellm_deprecation.DeprecationStatus
    ).json_schema()
    our_status: dict[str, JsonValue] = TypeAdapter(
        model_deprecation.DeprecationStatus
    ).json_schema()
    assert strip_volatile(our_status) == strip_volatile(their_status)
    their_provider: dict[str, JsonValue] = TypeAdapter(
        litellm_spend_capture.SpendCaptureProvider
    ).json_schema()
    our_provider: dict[str, JsonValue] = TypeAdapter(
        spend_capture.SpendCaptureProvider
    ).json_schema()
    assert strip_volatile(our_provider) == strip_volatile(their_provider)
    their_snapshot: dict[str, JsonValue] = TypeAdapter(
        litellm_gateway.GatewayRequestSnapshot
    ).json_schema()
    our_snapshot: dict[str, JsonValue] = TypeAdapter(
        gateway_requests.GatewayRequestSnapshot
    ).json_schema()
    assert strip_volatile(our_snapshot) == strip_volatile(their_snapshot)


def test_constants_names_match() -> None:
    assert callback_logs.MAX_CALLBACK_LOG_RECORDS == litellm_callback_logs.MAX_CALLBACK_LOG_RECORDS
    assert agent_skills.MAX_SKILL_NAME_LENGTH == litellm_agent_skills.MAX_SKILL_NAME_LENGTH
    assert (
        agent_skills.MAX_SKILL_DESCRIPTION_LENGTH
        == litellm_agent_skills.MAX_SKILL_DESCRIPTION_LENGTH
    )
    our_schema_url = agent_skills.AGENT_SKILLS_DISCOVERY_SCHEMA_URL
    their_schema_url = litellm_agent_skills.AGENT_SKILLS_DISCOVERY_SCHEMA_URL
    assert our_schema_url == their_schema_url
    our_warn_days = model_deprecation.DEFAULT_DEPRECATION_WARN_DAYS
    their_warn_days = litellm_deprecation.DEFAULT_DEPRECATION_WARN_DAYS
    assert our_warn_days == their_warn_days
    our_interval = model_deprecation.DEFAULT_DEPRECATION_CHECK_INTERVAL_SECONDS
    their_interval = litellm_deprecation.DEFAULT_DEPRECATION_CHECK_INTERVAL_SECONDS
    assert our_interval == their_interval
    our_idle_poll = model_deprecation.DEPRECATION_IDLE_POLL_SECONDS
    their_idle_poll = litellm_deprecation.DEPRECATION_IDLE_POLL_SECONDS
    assert our_idle_poll == their_idle_poll
    our_max_days = spend_capture.SPEND_CAPTURE_RATE_MAX_RANGE_DAYS
    their_max_days = litellm_spend_capture.SPEND_CAPTURE_RATE_MAX_RANGE_DAYS
    assert our_max_days == their_max_days


def test_dataclass_names_match() -> None:
    their_key = litellm_gateway.GatewayRequestKey(date="2026-01-01", category="chat", route="/v1")
    our_key = gateway_requests.GatewayRequestKey(date="2026-01-01", category="chat", route="/v1")
    assert our_key.date == their_key.date
    assert our_key.category == their_key.category
    assert our_key.route == their_key.route
    their_counts = litellm_gateway.GatewayRequestCounts(successful_requests=1, failed_requests=2)
    our_counts = gateway_requests.GatewayRequestCounts(successful_requests=1, failed_requests=2)
    their_success = their_counts.plus(succeeded=True)
    our_success = our_counts.plus(succeeded=True)
    assert our_success.successful_requests == their_success.successful_requests
    assert our_success.failed_requests == their_success.failed_requests
    their_failure = their_counts.plus(succeeded=False)
    our_failure = our_counts.plus(succeeded=False)
    assert our_failure.successful_requests == their_failure.successful_requests
    assert our_failure.failed_requests == their_failure.failed_requests


def test_ui_discovery_names() -> None:
    values: dict[str, JsonValue] = {
        "server_root_path": "/",
        "proxy_base_url": None,
        "auto_redirect_to_sso": False,
        "admin_ui_disabled": False,
        "sso_configured": False,
    }
    their_endpoints = litellm_ui_discovery.UiDiscoveryEndpoints.model_validate(values)
    our_endpoints = ui_discovery.UiDiscoveryEndpoints.model_validate(values)
    assert our_endpoints.model_dump() == their_endpoints.model_dump()
    assert our_endpoints.workers == []


def test_redacted_dict_names() -> None:
    headers: dict[str, JsonValue] = {"Authorization": "Bearer secret"}
    their_redacted = litellm_pre_call.RedactedDict(headers)
    our_redacted = RedactedDict(headers)
    assert repr(our_redacted) == repr(their_redacted)
    assert str(our_redacted) == str(their_redacted)
    assert our_redacted["Authorization"] == their_redacted["Authorization"]
    their_copy = their_redacted.copy()
    our_copy = our_redacted.copy()
    assert repr(our_copy) == repr(their_copy)
    our_copy["x"] = 1
    assert "x" not in our_redacted


def test_error_names_the_user() -> None:
    their_error = litellm_auth_checks.UserNotFoundError("user-1")
    our_error = auth_checks.UserNotFoundError("user-1")
    assert str(our_error) == str(their_error)


def test_budget_snapshot_names() -> None:
    metadata: dict[str, JsonValue] = {
        "user_api_key_budget_reset_at": "2026-01-01T00:00:00Z",
        "unrelated": "ignored",
    }
    their_snapshot = litellm_carried_budget.KeyBudgetSnapshot.from_metadata(metadata)
    our_snapshot = carried_budget.KeyBudgetSnapshot.from_metadata(metadata)
    assert their_snapshot is not None
    assert our_snapshot is not None
    assert dict(our_snapshot.metadata_entries()) == dict(their_snapshot.metadata_entries())
    invalid: dict[str, JsonValue] = {"user_api_key_budget_reset_at": "not-a-date"}
    assert litellm_carried_budget.KeyBudgetSnapshot.from_metadata(invalid) is None
    assert carried_budget.KeyBudgetSnapshot.from_metadata(invalid) is None


def test_entra_identity_names() -> None:
    values: dict[str, JsonValue] = {
        "provider": "microsoft_entra",
        "tenant_id": "00000000-0000-0000-0000-00000000000A",
        "client_id": "00000000-0000-0000-0000-00000000000B",
    }
    their_config = litellm_agent_identity.EntraIdentityConfig.model_validate(values)
    our_config = agent_identity.EntraIdentityConfig.model_validate(values)
    assert our_config.issuer == their_config.issuer
    assert our_config.tenant_id == their_config.tenant_id
    assert our_config.client_id == their_config.client_id
    invalid: dict[str, JsonValue] = {**values, "tenant_id": "nope"}
    with pytest.raises(ValidationError) as their_error:
        litellm_agent_identity.EntraIdentityConfig.model_validate(invalid)
    with pytest.raises(ValidationError) as our_error:
        agent_identity.EntraIdentityConfig.model_validate(invalid)
    assert our_error.value.errors()[0]["msg"] == their_error.value.errors()[0]["msg"]


def test_error_names_match() -> None:
    with pytest.raises(ValidationError) as their_spend:
        litellm_spend_capture.SpendCaptureRateCheckSettings(lookback_days=0)
    with pytest.raises(ValidationError) as our_spend:
        spend_capture.SpendCaptureRateCheckSettings(lookback_days=0)
    assert our_spend.value.errors()[0]["msg"] == their_spend.value.errors()[0]["msg"]
    with pytest.raises(ValidationError) as their_worker:
        litellm_control_plane.WorkerRegistryEntry(worker_id="w", name="n", url="ftp://worker")
    with pytest.raises(ValidationError) as our_worker:
        control_plane.WorkerRegistryEntry(worker_id="w", name="n", url="ftp://worker")
    assert our_worker.value.errors()[0]["msg"] == their_worker.value.errors()[0]["msg"]
    with pytest.raises(ValidationError) as their_vantage:
        litellm_vantage.VantageInitRequest(api_key=" ", integration_token="token")
    with pytest.raises(ValidationError) as our_vantage:
        vantage.VantageInitRequest(api_key=" ", integration_token="token")
    assert our_vantage.value.errors()[0]["msg"] == their_vantage.value.errors()[0]["msg"]
