from collections.abc import Mapping
from datetime import UTC, datetime
from typing import TypeAlias

import pytest

from betterlite.types.proxy.management_endpoints import (
    common_daily_activity,
    config_overrides,
    customer_endpoints,
    internal_user_endpoints,
    key_management_endpoints,
    management_v1,
    model_management_endpoints,
)
from betterlite.types.utils import JsonValue, ResponseObject, ValidationError

DumpValue: TypeAlias = JsonValue | tuple[str, ...]

EXPECTED_MAX_BULK_DELETE_USERS = 500

EXPECTED_MAX_BULK_NEW_USERS = 500

MODEL_GROUP_BOOL_FIELDS = (
    "supports_parallel_function_calling",
    "supports_vision",
    "supports_web_search",
    "supports_url_context",
    "supports_reasoning",
    "supports_function_calling",
    "supports_fast_mode",
    "is_public_model_group",
)

SPEND_AMOUNT = 1.5

SPEND_METRICS: dict[str, JsonValue] = {
    "spend": 0.0,
    "flat_cost": 0.0,
    "prompt_tokens": 0,
    "completion_tokens": 0,
    "cache_read_input_tokens": 0,
    "cache_creation_input_tokens": 0,
    "compression_saved_tokens": 0,
    "compression_savings_spend": 0.0,
    "prompt_caching_savings_spend": 0.0,
    "gateway_injected_caching_savings_spend": 0.0,
    "autorouter_savings_spend": 0.0,
    "total_tokens": 0,
    "successful_requests": 0,
    "failed_requests": 0,
    "api_requests": 0,
    "total_response_time_ms": 0,
    "timed_requests": 0,
}

KEY_METADATA: dict[str, JsonValue] = {
    "key_alias": None,
    "team_id": None,
    "user_id": None,
    "user_email": None,
    "key_exists": None,
}

BREAKDOWN_METRICS: dict[str, JsonValue] = {
    "mcp_servers": {},
    "models": {},
    "model_groups": {},
    "providers": {},
    "endpoints": {},
    "api_keys": {},
    "entities": {},
}

DAILY_METADATA: dict[str, JsonValue] = {
    "total_spend": 0.0,
    "total_flat_cost": 0.0,
    "total_prompt_tokens": 0,
    "total_completion_tokens": 0,
    "total_tokens": 0,
    "total_api_requests": 0,
    "total_successful_requests": 0,
    "total_failed_requests": 0,
    "total_cache_read_input_tokens": 0,
    "total_cache_creation_input_tokens": 0,
    "total_compression_saved_tokens": 0,
    "total_compression_savings_spend": 0.0,
    "total_prompt_caching_savings_spend": 0.0,
    "total_gateway_injected_caching_savings_spend": 0.0,
    "total_autorouter_savings_spend": 0.0,
    "total_response_time_ms": 0,
    "total_timed_requests": 0,
    "page": 1,
    "total_pages": 1,
    "has_more": False,
    "api_key_limit": None,
    "total_api_keys": None,
    "entity_total_api_keys": None,
}

KEY_SPEND: dict[str, JsonValue] = {
    "spend": 0.0,
    "prompt_tokens": 0,
    "completion_tokens": 0,
    "total_tokens": 0,
    "api_requests": 0,
    "successful_requests": 0,
    "failed_requests": 0,
    "cache_read_input_tokens": 0,
    "cache_creation_input_tokens": 0,
}

DAILY_USER_SPEND: dict[str, JsonValue] = {
    "mcp_server_id": None,
    "model": None,
    "model_group": None,
    "custom_llm_provider": None,
    "prompt_tokens": 0,
    "completion_tokens": 0,
    "cache_read_input_tokens": 0,
    "cache_creation_input_tokens": 0,
    "compression_saved_tokens": 0,
    "compression_savings_spend": 0.0,
    "prompt_caching_savings_spend": 0.0,
    "gateway_injected_caching_savings_spend": 0.0,
    "autorouter_savings_spend": 0.0,
    "spend": 0.0,
    "api_requests": 0,
    "successful_requests": 0,
    "failed_requests": 0,
    "total_response_time_ms": 0,
    "timed_requests": 0,
}

HASHICORP_VAULT: dict[str, JsonValue] = {
    "vault_addr": None,
    "vault_token": None,
    "approle_role_id": None,
    "approle_secret_id": None,
    "approle_mount_path": None,
    "client_cert": None,
    "client_key": None,
    "vault_cert_role": None,
    "vault_namespace": None,
    "vault_login_namespace": None,
    "vault_secret_namespace": None,
    "vault_mount_name": None,
    "vault_path_prefix": None,
}

CYBERARK: dict[str, JsonValue] = {
    "cyberark_api_base": None,
    "cyberark_account": None,
    "cyberark_username": None,
    "cyberark_api_key": None,
    "client_cert": None,
    "client_key": None,
    "ssl_verify": None,
    "refresh_interval": None,
}

CUSTOMER_RESPONSE: dict[str, JsonValue] = {
    "alias": None,
    "spend": 0.0,
    "allowed_model_region": None,
    "default_model": None,
    "budget_id": None,
    "litellm_budget_table": None,
    "object_permission_id": None,
    "object_permission": None,
}

BULK_UPDATE_USER: dict[str, JsonValue] = {
    "users": None,
    "all_users": False,
    "user_updates": None,
}

NEW_USER_ITEM: dict[str, JsonValue] = {
    "key_alias": None,
    "duration": None,
    "models": [],
    "spend": 0,
    "max_budget": None,
    "user_id": None,
    "team_id": None,
    "agent_id": None,
    "max_parallel_requests": None,
    "metadata": {},
    "tpm_limit": None,
    "rpm_limit": None,
    "budget_duration": None,
    "budget_limits": None,
    "allowed_cache_controls": [],
    "config": {},
    "permissions": {},
    "model_max_budget": {},
    "budget_fallbacks": None,
    "model_rpm_limit": None,
    "model_tpm_limit": None,
    "mcp_rpm_limit": None,
    "tag_rpm_limit": None,
    "guardrails": None,
    "policies": None,
    "prompts": None,
    "blocked": None,
    "aliases": {},
    "object_permission": None,
    "user_email": None,
    "user_alias": None,
    "user_role": None,
    "teams": None,
    "auto_create_key": False,
    "send_invite_email": None,
    "sso_user_id": None,
    "organizations": None,
    "password": None,
}

USER_UPDATE_RESULT: dict[str, JsonValue] = {
    "user_id": None,
    "user_email": None,
    "error": None,
    "updated_user": None,
}

USER_DELETE_RESULT: dict[str, DumpValue] = {
    "user_email": None,
    "teams_removed": (),
    "error": None,
}

USER_CREATE_RESULT: dict[str, JsonValue] = {
    "user_id": None,
    "user_email": None,
    "teams": None,
    "key": None,
    "error": None,
}

BULK_KEY_ITEM: dict[str, JsonValue] = {
    "budget_id": None,
    "max_budget": None,
    "team_id": None,
    "tags": None,
    "object_permission": None,
}

KEY_UPDATE_FIELDS: dict[str, JsonValue] = {
    "max_budget": None,
    "budget_id": None,
    "budget_duration": None,
    "budget_limits": None,
    "model_max_budget": None,
    "tpm_limit": None,
    "rpm_limit": None,
    "model_tpm_limit": None,
    "model_rpm_limit": None,
    "max_parallel_requests": None,
    "rpm_limit_type": None,
    "tpm_limit_type": None,
    "temp_budget_increase": None,
    "temp_budget_expiry": None,
    "duration": None,
    "tags": None,
    "metadata": None,
}

BULK_TEAM_KEYS: dict[str, JsonValue] = {
    "key_ids": None,
    "all_keys_in_team": False,
}

FAILED_KEY_UPDATE: dict[str, JsonValue] = {"key_info": None}

MODEL_GROUP_PROXY: dict[str, JsonValue] = {
    "max_input_tokens": None,
    "max_output_tokens": None,
    "input_cost_per_token": None,
    "output_cost_per_token": None,
    "input_cost_per_pixel": None,
    "mode": "chat",
    "tpm": None,
    "rpm": None,
    "itpm": None,
    "otpm": None,
    "supports_parallel_function_calling": False,
    "supports_vision": False,
    "supports_web_search": False,
    "supports_url_context": False,
    "supports_reasoning": False,
    "supports_function_calling": False,
    "supports_fast_mode": False,
    "supported_reasoning_efforts": None,
    "supported_openai_params": [],
    "configurable_clientside_auth_params": None,
    "is_public_model_group": False,
    "health_status": None,
    "health_response_time": None,
    "health_checked_at": None,
}

NEW_MODEL_GROUP: dict[str, JsonValue] = {"model_names": None, "model_ids": None}

GROUP_BUDGET: dict[str, JsonValue] = {
    "max_budget": None,
    "soft_budget": None,
    "budget_duration": None,
    "budget_reset_at": None,
}

GROUP_BUDGET_REQUEST: dict[str, JsonValue] = {
    "budget_id": None,
    "max_budget": None,
    "soft_budget": None,
    "budget_duration": None,
}

GROUP_INFO: dict[str, JsonValue] = {"spend": None, "budget": None}


def assert_model_names(
    model: type[ResponseObject],
    required: set[str],
    defaults: Mapping[str, DumpValue],
) -> None:
    fields = model.model_fields
    assert set(fields) == required | set(defaults)
    assert {name for name, field in fields.items() if field.is_required()} == required
    assert model.model_construct().model_dump() == defaults


def test_daily_activity_names() -> None:
    assert_model_names(common_daily_activity.SpendMetrics, set(), SPEND_METRICS)
    assert_model_names(common_daily_activity.MetricBase, {"metrics"}, {})
    assert_model_names(common_daily_activity.KeyMetadata, set(), KEY_METADATA)
    assert_model_names(
        common_daily_activity.KeyMetricWithMetadata,
        {"metrics"},
        {"metadata": KEY_METADATA},
    )
    assert_model_names(
        common_daily_activity.MetricWithMetadata,
        {"metrics"},
        {"metadata": {}, "api_key_breakdown": {}},
    )
    assert_model_names(common_daily_activity.BreakdownMetrics, set(), BREAKDOWN_METRICS)
    assert_model_names(
        common_daily_activity.DailySpendData,
        {"date", "metrics"},
        {"breakdown": BREAKDOWN_METRICS},
    )
    assert_model_names(common_daily_activity.DailySpendMetadata, set(), DAILY_METADATA)
    assert_model_names(
        common_daily_activity.SpendAnalyticsPaginatedResponse,
        {"results"},
        {"metadata": DAILY_METADATA},
    )
    assert_model_names(
        common_daily_activity.KeyActivityRow,
        {"api_key", "metrics", "metadata"},
        {},
    )
    assert_model_names(common_daily_activity.KeySpendMetrics, set(), KEY_SPEND)
    assert_model_names(
        common_daily_activity.KeySpendActivityRow,
        {"api_key", "metrics", "metadata"},
        {},
    )
    assert_model_names(
        common_daily_activity.DailyActivityKeySearchResponse,
        {"api_keys"},
        {},
    )
    assert_model_names(
        common_daily_activity.DailyActivityKeyPageResponse,
        {"api_keys", "total_api_keys", "offset", "limit"},
        {},
    )
    assert_model_names(
        common_daily_activity.ModelTopKeysResponse,
        {"model", "by_model_group", "api_keys"},
        {},
    )
    assert_model_names(common_daily_activity.CacheLeakageKeysResponse, {"api_keys"}, {})
    assert_model_names(
        common_daily_activity.DailyUserSpend,
        {"id", "user_id", "date", "api_key"},
        DAILY_USER_SPEND,
    )
    assert common_daily_activity.LiteLLM_DailyUserSpend is common_daily_activity.DailyUserSpend
    assert common_daily_activity.GroupedData.__optional_keys__ == frozenset()
    assert set(common_daily_activity.GroupedData.__required_keys__) == {
        "metrics",
        "breakdown",
    }
    dimensions = {
        member.name: member.value for member in common_daily_activity.GroupByDimension
    }
    assert dimensions == {
        "DATE": "date",
        "MODEL": "model",
        "API_KEY": "api_key",
        "TEAM": "team",
        "ORGANIZATION": "organization",
        "MODEL_GROUP": "model_group",
        "PROVIDER": "custom_llm_provider",
    }


def test_config_override_names() -> None:
    assert_model_names(config_overrides.HashicorpVaultConfig, set(), HASHICORP_VAULT)
    assert_model_names(config_overrides.CyberArkConfig, set(), CYBERARK)
    assert_model_names(
        config_overrides.ConfigOverrideSettingsResponse,
        {"config_type", "values", "field_schema"},
        {},
    )
    settings = config_overrides.ConfigOverrideSettingsResponse(
        config_type="vault",
        values={"vault_addr": "https://vault"},
        field_schema={"vault_addr": {"type": "string"}},
    )
    assert settings.config_type == "vault"
    assert settings.values == {"vault_addr": "https://vault"}
    missing: dict[str, JsonValue] = {"config_type": "vault"}
    with pytest.raises(ValidationError, match="values"):
        _ = config_overrides.ConfigOverrideSettingsResponse.model_validate(missing)


def test_customer_names() -> None:
    assert_model_names(
        customer_endpoints.CustomerResponse,
        {"user_id", "blocked"},
        CUSTOMER_RESPONSE,
    )
    assert_model_names(customer_endpoints.BlockUsersResponse, {"blocked_users"}, {})
    assert_model_names(customer_endpoints.UnblockUsersResponse, {"blocked_users"}, {})
    assert_model_names(
        customer_endpoints.DeleteCustomersResponse,
        {"deleted_customers", "message"},
        {},
    )
    european = customer_endpoints.CustomerResponse(
        user_id="u",
        blocked=False,
        allowed_model_region="eu",
    )
    assert european.allowed_model_region == "eu"
    american = customer_endpoints.CustomerResponse(
        user_id="u",
        blocked=False,
        allowed_model_region="us",
    )
    assert american.allowed_model_region == "us"
    invalid: dict[str, JsonValue] = {
        "user_id": "u",
        "blocked": False,
        "allowed_model_region": "cn",
    }
    with pytest.raises(ValidationError, match="allowed_model_region"):
        _ = customer_endpoints.CustomerResponse.model_validate(invalid)
    unblocked = customer_endpoints.UnblockUsersResponse(blocked_users=["a"])
    assert unblocked.blocked_users == ["a"]
    defaulted_payload: dict[str, JsonValue] = {
        "user_id": "u",
        "blocked": False,
        "spend": None,
    }
    defaulted = customer_endpoints.CustomerResponse.model_validate(defaulted_payload)
    assert defaulted.spend == 0.0
    spend_payload: dict[str, JsonValue] = {
        "user_id": "u",
        "blocked": False,
        "spend": SPEND_AMOUNT,
    }
    assert customer_endpoints.CustomerResponse.model_validate(
        spend_payload
    ).spend == SPEND_AMOUNT
    unknown_column: dict[str, JsonValue] = {
        "user_id": "u",
        "blocked": False,
        "unknown_column": "x",
    }
    filtered = customer_endpoints.CustomerResponse.model_validate(unknown_column)
    assert filtered.model_dump() == {"user_id": "u", "blocked": False, **CUSTOMER_RESPONSE}
    assert filtered.json() == filtered.model_dump()
    assert filtered.fields_set() == {"user_id", "blocked"}
    deleted = customer_endpoints.DeleteCustomersResponse(
        deleted_customers=2,
        message="deleted",
    )
    assert deleted.model_dump() == {"deleted_customers": 2, "message": "deleted"}


def test_internal_user_names() -> None:
    assert internal_user_endpoints.MAX_BULK_DELETE_USERS == EXPECTED_MAX_BULK_DELETE_USERS
    assert internal_user_endpoints.MAX_BULK_NEW_USERS == EXPECTED_MAX_BULK_NEW_USERS
    assert internal_user_endpoints.InsensitiveContains.__optional_keys__ == frozenset()
    assert set(internal_user_endpoints.InsensitiveContains.__required_keys__) == {
        "contains",
        "mode",
    }
    assert internal_user_endpoints.UserSearchWhere.__optional_keys__ == frozenset()
    assert set(internal_user_endpoints.UserSearchWhere.__required_keys__) == {"OR"}
    assert_model_names(
        internal_user_endpoints.UserListResponse,
        {"users", "total", "page", "page_size", "total_pages"},
        {},
    )
    assert_model_names(
        internal_user_endpoints.BulkUpdateUserRequest,
        set(),
        BULK_UPDATE_USER,
    )
    assert_model_names(
        internal_user_endpoints.UserUpdateResult,
        {"success"},
        USER_UPDATE_RESULT,
    )
    assert_model_names(
        internal_user_endpoints.BulkUpdateUserResponse,
        {"results", "total_requested", "successful_updates", "failed_updates"},
        {},
    )
    assert_model_names(
        internal_user_endpoints.BulkDeleteUserRequest,
        {"user_ids"},
        {},
    )
    assert_model_names(
        internal_user_endpoints.UserDeleteResult,
        {"user_id", "success"},
        USER_DELETE_RESULT,
    )
    assert_model_names(internal_user_endpoints.BulkDeleteUsersResponse, {"data"}, {})
    assert_model_names(internal_user_endpoints.BulkNewUserItem, set(), NEW_USER_ITEM)
    assert_model_names(
        internal_user_endpoints.BulkNewUserRequest,
        {"users"},
        {},
    )
    assert_model_names(
        internal_user_endpoints.UserCreateResult,
        {"success"},
        USER_CREATE_RESULT,
    )
    assert_model_names(
        internal_user_endpoints.BulkNewUserMeta,
        {"total_requested", "created", "failed"},
        {},
    )
    assert_model_names(
        internal_user_endpoints.BulkNewUserResponse,
        {"data", "meta"},
        {},
    )


def test_internal_user_names_rules() -> None:
    assert internal_user_endpoints.BulkUpdateUserRequest().users is None
    assert internal_user_endpoints.BulkUpdateUserRequest(
        users=[{"user_id": "u"}]
    ).users == [{"user_id": "u"}]
    assert internal_user_endpoints.BulkUpdateUserRequest(
        users=[], all_users=True, user_updates={"max_budget": 1.0}
    ).users == []
    valid = internal_user_endpoints.BulkUpdateUserRequest(
        all_users=True,
        user_updates={"max_budget": 1.0},
    )
    assert valid.all_users is True
    empty_updates = internal_user_endpoints.BulkUpdateUserRequest(
        all_users=True,
        user_updates={},
    )
    assert empty_updates.all_users is True
    assert empty_updates.user_updates == {}
    with pytest.raises(ValidationError, match="Must specify either"):
        _ = internal_user_endpoints.BulkUpdateUserRequest(user_updates={})
    with pytest.raises(ValidationError, match="Cannot specify both"):
        _ = internal_user_endpoints.BulkUpdateUserRequest(
            users=[{"user_id": "u"}],
            all_users=True,
            user_updates={"max_budget": 1.0},
        )
    with pytest.raises(ValidationError, match="Must specify either"):
        _ = internal_user_endpoints.BulkUpdateUserRequest(user_updates=None)
    user_ids = tuple(str(index) for index in range(EXPECTED_MAX_BULK_DELETE_USERS))
    accepted = internal_user_endpoints.BulkDeleteUserRequest(user_ids=user_ids)
    assert len(accepted.user_ids) == EXPECTED_MAX_BULK_DELETE_USERS
    with pytest.raises(ValidationError, match="user_ids"):
        _ = internal_user_endpoints.BulkDeleteUserRequest(user_ids=(*user_ids, "u"))
    with pytest.raises(ValidationError, match="user_ids"):
        _ = internal_user_endpoints.BulkDeleteUserRequest(user_ids=())
    forbidden: dict[str, JsonValue] = {"user_ids": ["u"], "extra": True}
    with pytest.raises(ValidationError, match="extra"):
        _ = internal_user_endpoints.BulkDeleteUserRequest.model_validate(forbidden)
    response_payload: dict[str, JsonValue] = {
        "data": [{"user_id": "u", "success": True}],
    }
    deleted = internal_user_endpoints.BulkDeleteUsersResponse.model_validate(
        response_payload
    )
    assert deleted.data[0].user_id == "u"
    assert deleted.data[0].teams_removed == ()


def test_internal_bulk_names_rules() -> None:
    one_item = internal_user_endpoints.BulkNewUserItem()
    assert one_item.json() == one_item.model_dump()
    assert one_item.fields_set() == set()
    many = internal_user_endpoints.BulkNewUserRequest(
        users=[one_item] * EXPECTED_MAX_BULK_NEW_USERS
    )
    assert len(many.users) == EXPECTED_MAX_BULK_NEW_USERS
    with pytest.raises(ValidationError, match="users"):
        _ = internal_user_endpoints.BulkNewUserRequest(
            users=[one_item] * (EXPECTED_MAX_BULK_NEW_USERS + 1)
        )
    with pytest.raises(ValidationError, match="users"):
        _ = internal_user_endpoints.BulkNewUserRequest(users=[])
    assert internal_user_endpoints.BulkNewUserItem(
        user_role="proxy_admin"
    ).user_role == "proxy_admin"
    assert internal_user_endpoints.BulkNewUserItem(
        user_role="proxy_admin_viewer"
    ).user_role == "proxy_admin_viewer"
    assert internal_user_endpoints.BulkNewUserItem(
        user_role="internal_user"
    ).user_role == "internal_user"
    assert internal_user_endpoints.BulkNewUserItem(
        user_role="internal_user_viewer"
    ).user_role == "internal_user_viewer"
    bad_role: dict[str, JsonValue] = {"user_role": "admin"}
    with pytest.raises(ValidationError, match="user_role"):
        _ = internal_user_endpoints.BulkNewUserItem.model_validate(bad_role)
    with pytest.raises(ValidationError, match="send_invite_email"):
        _ = internal_user_endpoints.BulkNewUserItem(send_invite_email=True)
    assert internal_user_endpoints.BulkNewUserItem(
        send_invite_email=False
    ).send_invite_email is False
    assert internal_user_endpoints.BulkNewUserItem(password=None).password is None
    password_payload: dict[str, JsonValue] = {"password": "secret"}
    with pytest.raises(ValidationError, match="password"):
        _ = internal_user_endpoints.BulkNewUserItem.model_validate(password_payload)
    unknown: dict[str, JsonValue] = {"bogus": 1}
    with pytest.raises(ValidationError, match="bogus"):
        _ = internal_user_endpoints.BulkNewUserItem.model_validate(unknown)
    request_payload: dict[str, JsonValue] = {"users": [{}], "bogus": 1}
    with pytest.raises(ValidationError, match="bogus"):
        _ = internal_user_endpoints.BulkNewUserRequest.model_validate(request_payload)
    bulk_item = internal_user_endpoints.BulkNewUserItem
    empty_budget: dict[str, JsonValue] = {"max_budget": ""}
    kept_budget: dict[str, JsonValue] = {"max_budget": SPEND_AMOUNT}
    assert bulk_item.model_validate(empty_budget).max_budget is None
    assert bulk_item.model_validate(kept_budget).max_budget == SPEND_AMOUNT
    team_payload: dict[str, JsonValue] = {"teams": [{"team_id": "t"}]}
    team_row = bulk_item.model_validate(team_payload).model_dump()["teams"]
    assert team_row == [
        {"team_id": "t", "max_budget_in_team": None, "user_role": "user"}
    ]
    empty_window: dict[str, JsonValue] = {"budget_limits": [{}]}
    with pytest.raises(ValidationError, match="budget_limits"):
        _ = bulk_item.model_validate(empty_window)
    permission_payload: dict[str, JsonValue] = {
        "object_permission": {"models": ["m"], "bogus": 1}
    }
    permission = bulk_item.model_validate(permission_payload).object_permission
    assert permission is not None
    assert "bogus" not in permission.model_dump()


def test_key_mgmt_names() -> None:
    keys = key_management_endpoints
    assert keys.KeyTokenWhere.__optional_keys__ == frozenset()
    assert set(keys.KeyTokenWhere.__required_keys__) == {"token"}
    assert keys.KeyAliasContainsWhere.__optional_keys__ == frozenset()
    assert set(keys.KeyAliasContainsWhere.__required_keys__) == {"key_alias"}
    assert keys.KeySearchWhere.__optional_keys__ == frozenset()
    assert set(keys.KeySearchWhere.__required_keys__) == {"OR"}
    assert_model_names(keys.BulkUpdateKeyRequestItem, {"key"}, BULK_KEY_ITEM)
    assert_model_names(keys.BulkUpdateKeyRequest, {"keys"}, {})
    assert_model_names(keys.SuccessfulKeyUpdate, {"key", "key_info"}, {})
    assert_model_names(
        keys.FailedKeyUpdate,
        {"key", "failed_reason"},
        FAILED_KEY_UPDATE,
    )
    assert_model_names(
        keys.BulkUpdateKeyResponse,
        {"total_requested", "successful_updates", "failed_updates"},
        {},
    )
    assert_model_names(keys.KeyUpdateFields, set(), KEY_UPDATE_FIELDS)
    assert_model_names(
        keys.BulkUpdateTeamKeysRequest,
        {"team_id", "update_fields"},
        BULK_TEAM_KEYS,
    )
    assert_model_names(
        keys.CustomKeyPolicyRequest,
        {"operation", "existing_key", "effective_key", "request"},
        {},
    )
    bulk = keys.BulkUpdateKeyRequest(keys=[keys.BulkUpdateKeyRequestItem(key="k")])
    assert bulk.keys[0].key == "k"


def test_key_mgmt_names_rules() -> None:
    keys = key_management_endpoints
    assert keys.KeyUpdateFields(max_budget=1.0).max_budget == 1.0
    with pytest.raises(ValidationError, match="at least one field"):
        _ = keys.KeyUpdateFields()
    expiry = datetime(2024, 1, 1, tzinfo=UTC)
    with pytest.raises(ValidationError, match="must be set together"):
        _ = keys.KeyUpdateFields(temp_budget_increase=1.0)
    with pytest.raises(ValidationError, match="must be set together"):
        _ = keys.KeyUpdateFields(temp_budget_expiry=expiry)
    granted = keys.KeyUpdateFields(
        temp_budget_increase=1.0,
        temp_budget_expiry=expiry,
    )
    assert granted.temp_budget_expiry == expiry
    extra_field: dict[str, JsonValue] = {"spend": 1.0}
    with pytest.raises(ValidationError, match="spend"):
        _ = keys.KeyUpdateFields.model_validate(extra_field)
    assert keys.KeyUpdateFields(
        rpm_limit_type="guaranteed_throughput"
    ).rpm_limit_type == "guaranteed_throughput"
    assert keys.KeyUpdateFields(
        rpm_limit_type="best_effort_throughput"
    ).rpm_limit_type == "best_effort_throughput"
    assert keys.KeyUpdateFields(
        rpm_limit_type="dynamic"
    ).rpm_limit_type == "dynamic"
    assert keys.KeyUpdateFields(
        tpm_limit_type="guaranteed_throughput"
    ).tpm_limit_type == "guaranteed_throughput"
    assert keys.KeyUpdateFields(
        tpm_limit_type="best_effort_throughput"
    ).tpm_limit_type == "best_effort_throughput"
    assert keys.KeyUpdateFields(
        tpm_limit_type="dynamic"
    ).tpm_limit_type == "dynamic"
    bad_limit: dict[str, JsonValue] = {"rpm_limit_type": "bogus"}
    with pytest.raises(ValidationError, match="rpm_limit_type"):
        _ = keys.KeyUpdateFields.model_validate(bad_limit)
    fields = keys.KeyUpdateFields(max_budget=1.0)
    selected = keys.BulkUpdateTeamKeysRequest(
        team_id="t",
        key_ids=["k"],
        update_fields=fields,
    )
    assert selected.key_ids == ["k"]
    with pytest.raises(ValidationError, match="not both"):
        _ = keys.BulkUpdateTeamKeysRequest(
            team_id="t",
            key_ids=["k"],
            all_keys_in_team=True,
            update_fields=fields,
        )
    with pytest.raises(ValidationError, match="Must provide either"):
        _ = keys.BulkUpdateTeamKeysRequest(team_id="t", update_fields=fields)
    with pytest.raises(ValidationError, match="Must provide either"):
        _ = keys.BulkUpdateTeamKeysRequest(
            team_id="t",
            key_ids=[],
            update_fields=fields,
        )
    every_key = keys.BulkUpdateTeamKeysRequest(
        team_id="t",
        all_keys_in_team=True,
        update_fields=fields,
    )
    assert every_key.all_keys_in_team is True
    policy = keys.CustomKeyPolicyRequest(
        operation="generate",
        existing_key=None,
        effective_key={},
        request={},
    )
    assert policy.operation == "generate"
    with pytest.raises(ValidationError, match="frozen"):
        policy.operation = "update"
    with pytest.raises(ValidationError, match="frozen"):
        policy["operation"] = "update"
    assert policy.operation == "generate"
    regenerated = keys.CustomKeyPolicyRequest(
        operation="regenerate",
        existing_key=None,
        effective_key={"token": "t"},
        request={},
    )
    assert regenerated.operation == "regenerate"
    updated = keys.CustomKeyPolicyRequest(
        operation="update",
        existing_key={"token": "t"},
        effective_key={"token": "t"},
        request={},
    )
    assert updated.operation == "update"
    bad_operation: dict[str, JsonValue] = {
        "operation": "delete",
        "existing_key": None,
        "effective_key": {},
        "request": {},
    }
    with pytest.raises(ValidationError, match="operation"):
        _ = keys.CustomKeyPolicyRequest.model_validate(bad_operation)
    item_payload: dict[str, JsonValue] = {"key": "k", "spend": 0}
    item = keys.BulkUpdateKeyRequestItem.model_validate(item_payload)
    assert item.model_dump(exclude_unset=True) == {"key": "k"}
    assert policy.json() == policy.model_dump()
    assert policy.fields_set() == {"operation", "existing_key", "effective_key", "request"}


def test_management_v1_names() -> None:
    assert_model_names(
        management_v1.ProblemDetail,
        {"type", "title", "status", "detail"},
        {"allowed": None},
    )
    assert_model_names(management_v1.PageLinks, {"self_link"}, {"prev": None, "next": None})
    assert_model_names(management_v1.PageMeta, {"page", "page_size", "has_more"}, {})
    assert_model_names(management_v1.FacetListResponse, {"data", "meta", "links"}, {})
    assert_model_names(
        management_v1.ListMeta,
        {"total_count", "page", "page_size", "total_pages"},
        {},
    )
    assert_model_names(
        management_v1.ListLinks,
        {"self_link", "first", "last"},
        {"prev": None, "next": None},
    )
    assert_model_names(management_v1.ResourceResponse, {"data"}, {})
    assert_model_names(management_v1.ListResponse, {"data", "meta", "links"}, {})
    links_payload: dict[str, JsonValue] = {"self": "s"}
    links = management_v1.PageLinks.model_validate(links_payload)
    assert links.self_link == "s"
    assert management_v1.PageLinks(self_link="s").self_link == "s"
    both_payload: dict[str, JsonValue] = {"self": "self", "self_link": "field"}
    assert management_v1.PageLinks.model_validate(both_payload).self_link == "self"
    assert links.model_dump(by_alias=True) == {
        "self": "s",
        "prev": None,
        "next": None,
    }
    missing_payload: dict[str, JsonValue] = {}
    with pytest.raises(ValidationError, match="self"):
        _ = management_v1.PageLinks.model_validate(missing_payload)
    list_payload: dict[str, JsonValue] = {"self": "s", "first": "f", "last": "l"}
    full_links = management_v1.ListLinks.model_validate(list_payload)
    assert full_links.first == "f"
    assert full_links.last == "l"
    assert full_links.model_dump(by_alias=True) == {
        "self": "s",
        "first": "f",
        "prev": None,
        "next": None,
        "last": "l",
    }
    problem = management_v1.ProblemDetail(
        type="about:blank",
        title="Bad Request",
        status=400,
        detail="detail",
    )
    assert problem.model_dump() == {
        "type": "about:blank",
        "title": "Bad Request",
        "status": 400,
        "detail": "detail",
        "allowed": None,
    }
    meta = management_v1.PageMeta(page=1, page_size=2, has_more=False)
    facet = management_v1.FacetListResponse(
        data=["a", "b"],
        meta=meta,
        links=management_v1.PageLinks(self_link="s"),
    )
    assert facet.data == ["a", "b"]
    list_meta = management_v1.ListMeta(
        total_count=1,
        page=1,
        page_size=1,
        total_pages=1,
    )
    listed = management_v1.ListResponse[str](
        data=["a"],
        meta=list_meta,
        links=management_v1.ListLinks(self_link="s", first="f", last="l"),
    )
    assert listed.data == ["a"]


def test_model_mgmt_names() -> None:
    models = model_management_endpoints
    assert_model_names(models.ModelGroupInfoProxy, {"model_group", "providers"}, MODEL_GROUP_PROXY)
    assert_model_names(models.UpdateUsefulLinksRequest, {"useful_links"}, {})
    assert_model_names(models.AutoRouterClassifierDefaultPromptResponse, {"system_prompt"}, {})
    assert_model_names(models.NewModelGroupRequest, {"access_group"}, NEW_MODEL_GROUP)
    assert_model_names(
        models.NewModelGroupResponse,
        {"access_group", "models_updated"},
        NEW_MODEL_GROUP,
    )
    assert_model_names(models.UpdateModelGroupRequest, set(), NEW_MODEL_GROUP)
    assert_model_names(
        models.DeleteModelGroupResponse,
        {"access_group", "models_updated", "message"},
        {},
    )
    assert_model_names(models.AccessGroupBudget, {"budget_id"}, GROUP_BUDGET)
    assert_model_names(models.AccessGroupBudgetRequest, set(), GROUP_BUDGET_REQUEST)
    assert_model_names(
        models.AccessGroupBudgetResponse,
        {"access_group", "spend"},
        {"budget": None},
    )
    assert_model_names(
        models.DeleteAccessGroupBudgetResponse,
        {"access_group", "budget_deleted", "message"},
        {},
    )
    assert_model_names(
        models.AccessGroupInfo,
        {"access_group", "model_names", "deployment_count"},
        GROUP_INFO,
    )
    assert_model_names(models.ListAccessGroupsResponse, {"access_groups"}, {})


def test_model_mgmt_names_rules() -> None:
    models = model_management_endpoints
    proxy = models.ModelGroupInfoProxy(model_group="g", providers=["p"])
    assert proxy.mode == "chat"
    for name in MODEL_GROUP_BOOL_FIELDS:
        assert proxy[name] is False
        assert name in proxy.model_fields_set
        fields: dict[str, JsonValue] = {
            "model_group": "g",
            "providers": ["p"],
            name: None,
        }
        coerced = models.ModelGroupInfoProxy(**fields)
        assert coerced[name] is False
        assert name in coerced.model_fields_set
    assert models.ModelGroupInfoProxy(
        model_group="g",
        providers=["p"],
        supports_vision=True,
    ).supports_vision is True
    links = models.UpdateUsefulLinksRequest(
        useful_links={"old": "https://a", "new": {"url": "https://b", "index": 1}}
    )
    assert links.useful_links["old"] == "https://a"
    budget = models.AccessGroupBudgetRequest(max_budget=0.0, soft_budget=1.5)
    assert budget.max_budget == 0.0
    with pytest.raises(ValidationError, match="max_budget"):
        _ = models.AccessGroupBudgetRequest(max_budget=-1.0)
    with pytest.raises(ValidationError, match="soft_budget"):
        _ = models.AccessGroupBudgetRequest(soft_budget=-1.0)
    extra_field: dict[str, JsonValue] = {"tpm_limit": 1}
    with pytest.raises(ValidationError, match="tpm_limit"):
        _ = models.AccessGroupBudgetRequest.model_validate(extra_field)
    empty_payload: dict[str, JsonValue] = {}
    with pytest.raises(ValidationError, match="access_group"):
        _ = models.NewModelGroupRequest.model_validate(empty_payload)
    group = models.NewModelGroupRequest(access_group="g")
    assert group.model_names is None
    prompt = models.AutoRouterClassifierDefaultPromptResponse(system_prompt="p")
    assert prompt.system_prompt == "p"
    deleted = models.DeleteAccessGroupBudgetResponse(
        access_group="g",
        budget_deleted=False,
        message="none",
    )
    assert deleted.budget_deleted is False
    response = models.AccessGroupBudgetResponse(access_group="g", spend=1.5)
    assert response.budget is None
    groups = models.ListAccessGroupsResponse(
        access_groups=[
            models.AccessGroupInfo(
                access_group="g",
                model_names=["m"],
                deployment_count=1,
            )
        ]
    )
    assert groups.access_groups[0].model_names == ["m"]
