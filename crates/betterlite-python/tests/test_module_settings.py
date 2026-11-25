import ast
import importlib
import inspect
import json
from collections.abc import Awaitable, Callable, Mapping
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from pathlib import Path
from types import ModuleType, NoneType
from typing import Final, TypeAlias, get_args, get_type_hints

import httpx2
import litellm
import pytest
from pydantic import JsonValue, TypeAdapter

import betterlite
import betterlite._settings.callbacks as settings_callbacks
import betterlite._settings.params as settings_params
from betterlite import _native
from betterlite._router_types_routing import ModelGroupSettings
from betterlite._settings.callbacks import Callback
from betterlite._settings.params import CacheReference, ProxyAuthReference
from betterlite.types.integrations import datadog, newrelic, pointfive, zerobus
from betterlite.types.integrations.datadog_llm_obs import DatadogLLMObsInitParams
from betterlite.types.proxy.management_endpoints import ui_sso
from betterlite.types.secret_managers.main import KeyManagementSystem
from betterlite.types.utils import BudgetConfig
from betterlite.types.utils.base import ModelValue

ASSIGNMENT_LIMIT = 2314
MIN_ASSIGNMENTS = 400
MIN_COST_ENTRIES = 1000
MIN_MODELS = 1000
MIN_PROVIDER_SETS = 100

STRINGS: Final = TypeAdapter(list[str])
MODEL_SETS: Final = TypeAdapter(set[str])
Rule: TypeAlias = Callable[[str], JsonValue | Awaitable[JsonValue] | Mapping[str, JsonValue]]

COST_MAP_URL = (
    "https://raw.githubusercontent.com/BerriAI/litellm/main/"
    "model_prices_and_context_window.json"
)
BLOG_URL = "https://docs.litellm.ai/blog/rss.xml"
BETA_HEADERS_URL = (
    "https://raw.githubusercontent.com/BerriAI/litellm/main/"
    "litellm/anthropic_beta_headers_config.json"
)
PRESETS_URL = (
    "https://raw.githubusercontent.com/BerriAI/litellm/main/"
    "litellm/proxy/public_endpoints/autorouter_presets.json"
)

SETTING_NAMES: tuple[str, ...] = (
    "litellm_mode", "input_callback", "success_callback", "failure_callback", "service_callback",
    "audit_log_callbacks", "_custom_logger_compatible_callbacks_literal",
    "cold_storage_custom_logger", "logged_real_time_event_types",
    "_known_custom_logger_compatible_callbacks", "callbacks", "callback_settings",
    "initialized_langfuse_clients", "langfuse_default_tags", "langfuse_enable_update_trace_keys",
    "langsmith_batch_size", "prometheus_initialize_budget_metrics", "prometheus_latency_buckets",
    "require_auth_for_metrics_endpoint", "argilla_batch_size", "datadog_use_v1",
    "gcs_pub_sub_use_v1", "generic_api_use_v1", "argilla_transformation_object",
    "_async_input_callback", "_async_success_callback", "_async_failure_callback", "pre_call_rules",
    "post_call_rules", "turn_off_message_logging", "standard_logging_payload_excluded_fields",
    "log_raw_request_response", "log_client_error_tracebacks", "log_auth_failure_key_identity",
    "request_correlation_in_logs", "redact_messages_in_exceptions", "redact_user_api_key_info",
    "expose_router_debug_in_errors", "filter_invalid_headers",
    "add_user_information_to_llm_headers", "overwrite_user_with_key_hash",
    "bedrock_request_metadata_fields", "store_audit_logs", "skip_system_message_in_guardrail",
    "skip_tool_message_in_guardrail", "email", "token", "max_tokens", "drop_params",
    "modify_params", "bedrock_neutralize_orphaned_tool_blocks",
    "use_chat_completions_url_for_anthropic_messages", "strip_anthropic_total_tokens",
    "anthropic_sse_ping_interval_seconds", "sse_keepalive_ping_interval_seconds",
    "route_all_chat_openai_to_responses", "gemini_live_defer_setup", "retry", "api_key",
    "openai_key", "groq_key", "gigachat_key", "xai_key", "databricks_key", "openai_like_key",
    "azure_key", "anthropic_key", "replicate_key", "bytez_key", "gdc_key", "gdc_api_base",
    "cohere_key", "infinity_key", "clarifai_key", "maritalk_key", "ai21_key", "ollama_key",
    "openrouter_key", "datarobot_key", "predibase_key", "huggingface_key", "vertex_project",
    "vertex_location", "predibase_tenant_id", "togetherai_api_key", "cloudflare_api_key",
    "vercel_ai_gateway_key", "baseten_key", "llama_api_key", "aleph_alpha_key", "nlp_cloud_key",
    "novita_api_key", "snowflake_key", "gradient_ai_api_key", "nebius_key", "wandb_key",
    "heroku_key", "cometapi_key", "ovhcloud_key", "lemonade_key", "sap_service_key",
    "amazon_nova_api_key", "inception_key", "common_cloud_provider_auth_params",
    "use_litellm_proxy", "use_client", "ssl_verify", "ssl_security_level", "ssl_certificate",
    "user_url_validation", "user_url_allowed_hosts", "provider_url_destination_allowed_hosts",
    "otel_tenant_destination_mode", "ssl_ecdh_curve", "disable_streaming_logging",
    "disable_token_counter", "disable_add_transform_inline_image_block",
    "disable_add_user_agent_to_request_tags", "disable_anthropic_gemini_context_caching_transform",
    "enable_anthropic_prompt_caching", "_anthropic_prompt_caching_ttl_env",
    "anthropic_prompt_caching_ttl", "openai_system_messages_first",
    "disable_vertex_batch_output_transformation", "extra_spend_tag_headers", "safe_memory_mode",
    "enable_azure_ad_token_refresh", "proxy_auth", "AZURE_DEFAULT_API_VERSION",
    "WATSONX_DEFAULT_API_VERSION", "COHERE_DEFAULT_EMBEDDING_INPUT_TYPE", "credential_list",
    "llamaguard_model_name", "openai_moderations_model_name", "presidio_ad_hoc_recognizers",
    "google_moderation_confidence_threshold", "llamaguard_unsafe_content_categories",
    "blocked_user_list", "banned_keywords_list", "llm_guard_mode", "guardrail_name_config_map",
    "include_cost_in_streaming_usage", "reasoning_auto_summary", "prompt_name_config_map",
    "enable_preview_features", "return_response_headers", "enable_json_schema_validation",
    "enable_model_config_credential_overrides", "enable_key_alias_format_validation",
    "key_alias_pattern", "enable_gemini_default_thinking_level_low", "logging",
    "enable_loadbalancing_on_batch_endpoints", "require_managed_files",
    "enable_caching_on_provider_specific_optional_params", "caching_with_models", "cache",
    "default_in_memory_ttl", "default_redis_ttl", "default_redis_batch_cache_expiry",
    "model_alias_map", "model_group_settings", "max_budget", "budget_exceeded_status_code",
    "budget_duration", "default_soft_budget", "budget_exceeded_throttle_percentage",
    "forward_traceparent_to_llm_provider", "_current_cost", "error_logs", "add_function_to_prompt",
    "client_session", "aclient_session", "model_fallbacks", "model_cost_map_url", "blog_posts_url",
    "anthropic_beta_headers_url", "autorouter_presets_url", "suppress_debug_info",
    "dynamodb_table_name", "s3_callback_params", "s3_audit_callback_params",
    "datadog_llm_observability_params", "datadog_params", "newrelic_params", "pointfive_params",
    "zerobus_params", "aws_sqs_callback_params", "generic_logger_headers",
    "default_key_generate_params", "default_key_max_budget_alert_emails",
    "upperbound_key_generate_params", "key_generation_settings", "default_internal_user_params",
    "default_team_params", "default_team_settings", "max_user_budget",
    "default_max_internal_user_budget", "max_internal_user_budget", "max_ui_session_budget",
    "internal_user_budget_duration", "budget_rollover", "tag_budget_config", "max_end_user_budget",
    "max_end_user_budget_id", "validate_end_user_id_in_db",
    "block_requests_for_models_without_pricing", "disable_end_user_cost_tracking",
    "disable_end_user_cost_tracking_prometheus_only",
    "enable_end_user_cost_tracking_prometheus_only", "custom_prometheus_metadata_labels",
    "custom_prometheus_tags", "prometheus_metrics_config", "prometheus_exclude_metrics",
    "prometheus_exclude_labels", "prometheus_emit_stream_label",
    "prometheus_emit_input_sequence_length_label",
    "prometheus_deployment_and_latency_caller_identity", "prometheus_emit_rate_limit_labels",
    "prometheus_user_budget_label_include_email_alias",
    "prometheus_end_user_metrics_max_series_per_metric", "prometheus_end_user_metrics_ttl_seconds",
    "prometheus_end_user_metrics_cleanup_interval_seconds", "disable_add_prefix_to_prompt",
    "disable_copilot_system_to_assistant", "public_mcp_servers", "public_mcp_hub_strict_whitelist",
    "public_model_groups", "public_skills_index", "public_agent_groups",
    "agent_search_embedding_model", "mcp_tool_search", "skill_search_embedding_model",
    "public_model_groups_links", "priority_reservation", "use_aiohttp_transport",
    "aiohttp_trust_env", "disable_aiohttp_transport", "disable_aiohttp_trust_env", "force_ipv4",
    "http2", "network_mock", "disable_stop_sequence_limit", "num_retries", "max_fallbacks",
    "default_fallbacks", "fallbacks", "context_window_fallbacks", "content_policy_fallbacks",
    "allowed_fails", "allow_dynamic_callback_disabling", "num_retries_per_request",
    "secret_manager_client", "_google_kms_resource_name", "_key_management_system",
    "output_parse_pii", "model_cost", "cost_discount_config", "cost_margin_config",
    "custom_prompt_dict", "check_provider_endpoint", "api_base", "headers", "api_version",
    "organization", "project", "config_path", "vertex_ai_safety_settings",
    "open_ai_chat_completion_models", "open_ai_text_completion_models", "cohere_models",
    "cohere_chat_models", "mistral_chat_models", "text_completion_codestral_models",
    "text_completion_inception_models", "anthropic_models", "openrouter_models", "datarobot_models",
    "vertex_language_models", "vertex_vision_models", "vertex_chat_models",
    "vertex_code_chat_models", "vertex_ai_image_models", "vertex_ai_video_models",
    "vertex_text_models", "vertex_code_text_models", "vertex_embedding_models",
    "vertex_anthropic_models", "vertex_llama3_models", "vertex_deepseek_models",
    "vertex_ai_ai21_models", "vertex_mistral_models", "vertex_openai_models",
    "vertex_minimax_models", "vertex_moonshot_models", "vertex_zai_models", "ai21_models",
    "ai21_chat_models", "nlp_cloud_models", "aleph_alpha_models", "bedrock_models",
    "bedrock_converse_models", "fal_ai_models", "fireworks_ai_models",
    "fireworks_ai_embedding_models", "deepinfra_models", "perplexity_models", "watsonx_models",
    "gemini_models", "xai_models", "zai_models", "deepseek_models", "tencent_models",
    "runwayml_models", "azure_ai_models", "jina_ai_models", "voyage_models", "infinity_models",
    "heroku_models", "databricks_models", "cloudflare_models", "codestral_models",
    "friendliai_models", "featherless_ai_models", "palm_models", "groq_models", "azure_models",
    "azure_anthropic_models", "azure_text_models", "anyscale_models", "cerebras_models",
    "nadir_models", "galadriel_models", "nvidia_nim_models", "nvidia_riva_models", "soniox_models",
    "sambanova_models", "sambanova_embedding_models", "novita_models", "assemblyai_models",
    "snowflake_models", "gradient_ai_models", "llama_models", "nscale_models", "nebius_models",
    "nebius_embedding_models", "aiml_models", "deepgram_models", "elevenlabs_models",
    "dashscope_models", "qwencloud_models", "qwen_ai_platform_models", "moonshot_models",
    "publicai_models", "darkbloom_models", "v0_models", "morph_models", "lambda_ai_models",
    "inception_models", "hyperbolic_models", "black_forest_labs_models", "recraft_models",
    "cometapi_models", "oci_models", "vercel_ai_gateway_models", "edenai_models",
    "volcengine_models", "wandb_models", "ovhcloud_models", "ovhcloud_embedding_models",
    "lemonade_models", "docker_model_runner_models", "amazon_nova_models", "stability_models",
    "github_copilot_models", "chatgpt_models", "minimax_models", "aws_polly_models",
    "transcribe_models", "gigachat_models", "llamagate_models", "reducto_models",
    "bedrock_mantle_models", "azure_llms", "azure_embedding_models", "petals_models",
    "ollama_models", "maritalk_models", "model_list_set", "models_by_provider",
    "longer_context_model_fallback_dict", "all_embedding_models", "openai_image_generation_models",
    "openai_video_generation_models", "adapters", "vector_store_registry",
    "vector_store_index_registry", "vector_store_search_failure_mode", "custom_provider_map",
    "_custom_providers", "disable_hf_tokenizer_download", "global_disable_no_log_param",
    "global_bitbucket_config", "global_gitlab_config",
)

IMPORTED_SETTINGS: tuple[str, ...] = (
    "replicate_models", "clarifai_models", "huggingface_models", "modelscope_models",
    "empower_models", "together_ai_models", "baseten_models", "open_ai_embedding_models",
    "cohere_embedding_models", "bedrock_embedding_models",
)

SORTED_SETTINGS: tuple[str, ...] = ("model_list",)

# litellm.caching is its subpackage at runtime and _thread_context a threading.local.
NOT_COMPARED: tuple[str, ...] = (
    "caching",
    "_thread_context",
    # pytest probes __test__ on module attributes, so litellm's __getattr__ flips the flag.
    "_async_client_cleanup_registered",
)

PLACEHOLDER_SETTINGS: dict[str, str] = {
    "CALLBACK_TYPES": "a typing alias over CustomLogger, which its own task builds",
    "in_memory_llm_clients_cache": "an LLMClientCache instance from LiteLLM's caching code",
    "_key_management_settings": "a KeyManagementSettings instance from the key management code",
    "vertexAITextEmbeddingConfig": "a class holding the Vertex embedding config",
}

NOT_BUILT: tuple[str, ...] = ("_AGENT_EXPORTS",)

ASSIGNMENT_GROUPS: tuple[tuple[str, ...], ...] = (
    SETTING_NAMES,
    SORTED_SETTINGS,
    NOT_COMPARED,
    tuple(PLACEHOLDER_SETTINGS),
    NOT_BUILT,
)

DECLARED_ARGUMENTS: Final[dict[str, tuple[type, ...]]] = {
    "Optional[str]": (str, NoneType),
    "Optional[float]": (float, NoneType),
    "Optional[List]": (list[JsonValue], NoneType),
    "Optional[Dict]": (dict[str, JsonValue], NoneType),
    "Optional[Dict[str, list]]": (dict[str, list[JsonValue]], NoneType),
    "Optional[List[str]]": (list[str], NoneType),
    "Optional[bool]": (bool, NoneType),
    "Optional[int]": (int, NoneType),
    "Optional[Union[str, List]]": (str, list[JsonValue], NoneType),
    "Optional[dict]": (dict[str, JsonValue], NoneType),
    "str | None": (str, NoneType),
    "float | None": (float, NoneType),
}

SPECIAL_ARGUMENTS: Final[dict[str, tuple[type, ...]]] = {
    "_key_management_system": (KeyManagementSystem, NoneType),
    "aclient_session": (httpx2.AsyncClient, NoneType),
    "cache": (CacheReference, NoneType),
    "client_session": (httpx2.Client, NoneType),
    "datadog_llm_observability_params": (DatadogLLMObsInitParams, dict[str, JsonValue], NoneType),
    "datadog_params": (datadog.DatadogInitParams, dict[str, JsonValue], NoneType),
    "default_team_params": (ui_sso.DefaultTeamSSOParams, dict[str, JsonValue], NoneType),
    "key_generation_settings": (object, NoneType),
    "mcp_tool_search": (Mapping[str, JsonValue], NoneType),
    "model_group_settings": (ModelGroupSettings, NoneType),
    "newrelic_params": (newrelic.NewRelicInitParams, dict[str, JsonValue], NoneType),
    "pointfive_params": (pointfive.PointFiveInitParams, Mapping[str, JsonValue], NoneType),
    "priority_reservation": (dict[str, object], NoneType),
    "proxy_auth": (ProxyAuthReference, NoneType),
    "secret_manager_client": (object, NoneType),
    "tag_budget_config": (dict[str, BudgetConfig], NoneType),
    "upperbound_key_generate_params": (ui_sso.UpperboundKeyGenerateParams, NoneType),
    "zerobus_params": (zerobus.ZerobusInitParams, Mapping[str, JsonValue], NoneType),
}

# in_memory_llm_clients_cache carries JsonValue, a type alias rather than a class
ALIASED_SETTINGS: tuple[str, ...] = ("in_memory_llm_clients_cache",)

ENV_VARIABLES: tuple[str, ...] = (
    "LITELLM_MODE",
    "DEFAULT_MAX_TOKENS",
    "DEFAULT_SOFT_BUDGET",
    "LITELLM_DROP_PARAMS",
    "LITELLM_MODIFY_PARAMS",
    "LITELLM_USE_CHAT_COMPLETIONS_URL_FOR_ANTHROPIC_MESSAGES",
    "LITELLM_ROUTE_ALL_CHAT_OPENAI_TO_RESPONSES",
    "LITELLM_GEMINI_LIVE_DEFER_SETUP",
    "LITELLM_ENABLE_ANTHROPIC_PROMPT_CACHING",
    "LITELLM_ANTHROPIC_PROMPT_CACHING_TTL",
    "LITELLM_MODEL_COST_MAP_URL",
    "LITELLM_BLOG_POSTS_URL",
    "LITELLM_ANTHROPIC_BETA_HEADERS_URL",
    "LITELLM_AUTOROUTER_PRESETS_URL",
    "LITELLM_DEV_ENV_HOT_RELOAD",
)

ENV_CASES: tuple[tuple[str, str, str, JsonValue], ...] = (
    ("LITELLM_DROP_PARAMS", "drop_params", "true", True),
    ("LITELLM_DROP_PARAMS", "drop_params", "false", False),
    ("LITELLM_DROP_PARAMS", "drop_params", "1", True),
    ("LITELLM_MODIFY_PARAMS", "modify_params", "true", True),
    (
        "LITELLM_USE_CHAT_COMPLETIONS_URL_FOR_ANTHROPIC_MESSAGES",
        "use_chat_completions_url_for_anthropic_messages",
        "true",
        True,
    ),
    (
        "LITELLM_ROUTE_ALL_CHAT_OPENAI_TO_RESPONSES",
        "route_all_chat_openai_to_responses",
        "TRUE",
        True,
    ),
    ("LITELLM_GEMINI_LIVE_DEFER_SETUP", "gemini_live_defer_setup", "false", False),
    ("LITELLM_ENABLE_ANTHROPIC_PROMPT_CACHING", "enable_anthropic_prompt_caching", "true", True),
    ("LITELLM_ANTHROPIC_PROMPT_CACHING_TTL", "anthropic_prompt_caching_ttl", "1h", "1h"),
    ("LITELLM_ANTHROPIC_PROMPT_CACHING_TTL", "anthropic_prompt_caching_ttl", "5m", "5m"),
    ("LITELLM_ANTHROPIC_PROMPT_CACHING_TTL", "anthropic_prompt_caching_ttl", "10m", None),
    ("DEFAULT_MAX_TOKENS", "max_tokens", "2048", 2048),
    ("DEFAULT_SOFT_BUDGET", "default_soft_budget", "12.5", 12.5),
    (
        "LITELLM_MODEL_COST_MAP_URL",
        "model_cost_map_url",
        "https://example.test/cost.json",
        "https://example.test/cost.json",
    ),
    (
        "LITELLM_BLOG_POSTS_URL",
        "blog_posts_url",
        "https://example.test/blog.xml",
        "https://example.test/blog.xml",
    ),
    (
        "LITELLM_ANTHROPIC_BETA_HEADERS_URL",
        "anthropic_beta_headers_url",
        "https://example.test/beta.json",
        "https://example.test/beta.json",
    ),
    (
        "LITELLM_AUTOROUTER_PRESETS_URL",
        "autorouter_presets_url",
        "https://example.test/presets.json",
        "https://example.test/presets.json",
    ),
)


def _members(module: ModuleType) -> dict[str, JsonValue]:
    members: list[tuple[str, JsonValue]] = inspect.getmembers(module)
    return dict(members)


def _thread_context_user() -> str:
    context = _members(betterlite)["_thread_context"]
    context_members: list[tuple[str, JsonValue]] = inspect.getmembers(context)
    return TypeAdapter(str).validate_python(dict(context_members)["user"])


def _identify_bob_in_this_thread() -> str:
    betterlite.identify({"user": "bob"})
    return _thread_context_user()


def _input_callback(kwargs: dict[str, JsonValue]) -> JsonValue:
    if not kwargs:
        return None
    return kwargs


def _event_callback(
    kwargs: dict[str, JsonValue],
    response_obj: ModelValue,
    start_time: datetime,
    end_time: datetime,
) -> JsonValue:
    return {
        "kwargs": kwargs,
        "response": str(response_obj),
        "start": start_time.isoformat(),
        "delta": (end_time - start_time).total_seconds(),
    }


def _call_rule(text: str) -> bool:
    refused = "cannot answer" in text
    return not refused


# ast-grep-ignore: no-long-names
def test_setting_defaults_match_litellm() -> None:
    ours = _members(betterlite)
    theirs = _members(litellm)
    names = (*SETTING_NAMES, *IMPORTED_SETTINGS)
    missing = [name for name in names if name not in ours or name not in theirs]
    assert missing == []
    unbound = [name for name in (*NOT_COMPARED, *PLACEHOLDER_SETTINGS) if name not in ours]
    assert unbound == []
    assert ours["_async_client_cleanup_registered"] is False
    differing = [name for name in names if ours[name] != theirs[name]]
    assert differing == []


# ast-grep-ignore: no-long-names
def test_setting_defaults_cover_litellm_assignments() -> None:
    source = Path(inspect.getfile(litellm)).read_text()
    assigned: set[str] = set()
    for node in ast.parse(source).body:
        if node.lineno > ASSIGNMENT_LIMIT:
            continue
        match node:
            case ast.Assign(targets=targets):
                for target in targets:
                    match target:
                        case ast.Name(id=name):
                            assigned.add(name)
                        case ast.expr():
                            continue
            case ast.AnnAssign(target=ast.Name(id=name)):
                assigned.add(name)
            case ast.stmt():
                continue
    listed = [name for group in ASSIGNMENT_GROUPS for name in group]
    assert len(listed) == len(set(listed))
    assert set(listed) == assigned
    assert len(assigned) > MIN_ASSIGNMENTS


# ast-grep-ignore: no-long-names
def test_setting_annotations_match_litellm_declarations() -> None:
    theirs: dict[str, str] = litellm.__annotations__
    ours: dict[str, type] = settings_params.__annotations__
    members = _members(settings_params)
    names = sorted(name for name, value in members.items() if value is None)
    bare = [name for name in names if name in theirs and name not in ours]
    assert bare == [], bare
    stray = [name for name in names if name not in theirs and name in ours]
    assert stray == [], stray
    for name in names:
        if name not in theirs or name in ALIASED_SETTINGS:
            continue
        if name in SPECIAL_ARGUMENTS:
            expected = SPECIAL_ARGUMENTS[name]
        else:
            expected = DECLARED_ARGUMENTS[theirs[name]]
        arguments: tuple[type, ...] = get_args(ours[name])
        assert arguments == expected, name


# ast-grep-ignore: no-long-names
def test_env_default_values_follow_the_environment(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("LITELLM_MODE", "PRODUCTION")
    for variable, setting, value, expected in ENV_CASES:
        monkeypatch.setenv(variable, value)
        importlib.reload(betterlite)
        assert _members(betterlite)[setting] == expected
        monkeypatch.delenv(variable)
    monkeypatch.undo()
    importlib.reload(betterlite)


def test_env_default_unset_values(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.chdir(tmp_path)
    for variable in ENV_VARIABLES:
        monkeypatch.delenv(variable, raising=False)
    importlib.reload(betterlite)
    members = _members(betterlite)
    expected: dict[str, JsonValue] = {
        "litellm_mode": "DEV",
        "max_tokens": 4096,
        "drop_params": False,
        "modify_params": False,
        "use_chat_completions_url_for_anthropic_messages": False,
        "route_all_chat_openai_to_responses": False,
        "gemini_live_defer_setup": False,
        "enable_anthropic_prompt_caching": False,
        "_anthropic_prompt_caching_ttl_env": None,
        "anthropic_prompt_caching_ttl": None,
        "default_soft_budget": 50.0,
        "model_cost_map_url": COST_MAP_URL,
        "blog_posts_url": BLOG_URL,
        "anthropic_beta_headers_url": BETA_HEADERS_URL,
        "autorouter_presets_url": PRESETS_URL,
    }
    assert {name: members[name] for name in expected} == expected
    monkeypatch.undo()
    importlib.reload(betterlite)


# ast-grep-ignore: no-long-names
def test_env_default_mode_follows_the_environment(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("LITELLM_MODE", "PRODUCTION")
    importlib.reload(betterlite)
    assert _members(betterlite)["litellm_mode"] == "PRODUCTION"
    monkeypatch.delenv("LITELLM_MODE")
    importlib.reload(betterlite)
    assert _members(betterlite)["litellm_mode"] == "DEV"
    monkeypatch.undo()
    importlib.reload(betterlite)


# ast-grep-ignore: no-long-names
def test_env_default_invalid_boolean_fails_the_load(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("LITELLM_MODIFY_PARAMS", "sometimes")
    with pytest.raises(ValueError, match="LITELLM_MODIFY_PARAMS"):
        importlib.reload(betterlite)
    monkeypatch.undo()
    importlib.reload(betterlite)


# ast-grep-ignore: no-long-names
def test_env_default_invalid_integer_fails_the_load(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("DEFAULT_MAX_TOKENS", "many")
    with pytest.raises(ValueError, match="DEFAULT_MAX_TOKENS"):
        importlib.reload(betterlite)
    monkeypatch.undo()
    importlib.reload(betterlite)


@pytest.mark.parametrize(
    ("mode", "expected"),
    [("DEV", "https://file.test/blog.xml"), ("PRODUCTION", BLOG_URL)],
)
def test_env_default_dotenv_modes(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path, mode: str, expected: str
) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.setenv("LITELLM_MODE", mode)
    monkeypatch.delenv("LITELLM_BLOG_POSTS_URL", raising=False)
    (tmp_path / ".env").write_text("LITELLM_BLOG_POSTS_URL=https://file.test/blog.xml\n")
    importlib.reload(betterlite)
    assert _members(betterlite)["blog_posts_url"] == expected
    monkeypatch.undo()
    importlib.reload(betterlite)


# ast-grep-ignore: no-long-names
def test_env_default_dotenv_overrides_only_with_hot_reload(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.delenv("LITELLM_MODE", raising=False)
    monkeypatch.setenv("LITELLM_DROP_PARAMS", "false")
    monkeypatch.setenv("LITELLM_DEV_ENV_HOT_RELOAD", "false")
    (tmp_path / ".env").write_text("LITELLM_DROP_PARAMS=true\n")
    importlib.reload(betterlite)
    assert _members(betterlite)["drop_params"] is False
    monkeypatch.setenv("LITELLM_DEV_ENV_HOT_RELOAD", "True")
    importlib.reload(betterlite)
    assert _members(betterlite)["drop_params"] is True
    monkeypatch.undo()
    importlib.reload(betterlite)


# ast-grep-ignore: no-long-names
def test_model_data_model_cost_matches_the_pin() -> None:
    bundled: dict[str, JsonValue] = json.loads(_native.bundled_model_cost_json())
    bundled.pop("fallback_generalizations", None)
    assert betterlite.model_cost == bundled
    assert len(betterlite.model_cost) > MIN_COST_ENTRIES


# ast-grep-ignore: no-long-names
def test_model_data_provider_sets_match_litellm() -> None:
    ours = _members(betterlite)
    theirs = _members(litellm)
    native = _native.provider_model_sets()
    assert len(native) > MIN_PROVIDER_SETS
    for name in native:
        assert MODEL_SETS.validate_python(ours[name]) == MODEL_SETS.validate_python(theirs[name])


# ast-grep-ignore: no-long-names
def test_model_data_model_list_matches_litellm() -> None:
    ours = STRINGS.validate_python(_members(betterlite)["model_list"])
    theirs = STRINGS.validate_python(_members(litellm)["model_list"])
    assert sorted(ours) == sorted(theirs)
    assert len(ours) > MIN_MODELS


# ast-grep-ignore: no-long-names
def test_model_data_native_data_stays_put(monkeypatch: pytest.MonkeyPatch) -> None:
    before = _native.provider_model_sets()
    monkeypatch.setitem(betterlite.model_cost, "probe-model", "probe")
    original = betterlite.open_ai_chat_completion_models
    betterlite.open_ai_chat_completion_models = {"probe-model"}
    assert _native.provider_model_sets() == before
    bundled: dict[str, JsonValue] = json.loads(_native.bundled_model_cost_json())
    assert "probe-model" not in bundled
    betterlite.open_ai_chat_completion_models = original


# ast-grep-ignore: no-long-names
def test_model_data_empty_lists_are_separate_objects() -> None:
    betterlite.input_callback.append("probe")
    assert betterlite.success_callback == []
    betterlite.input_callback.pop()
    betterlite.success_callback.append("probe")
    assert betterlite.input_callback == []
    betterlite.success_callback.pop()


# ast-grep-ignore: no-long-names
def test_module_helper_callback_lists_accept_typed_functions() -> None:
    hints: dict[str, type] = get_type_hints(settings_callbacks)
    assert hints["input_callback"] == list[Callback]
    assert hints["success_callback"] == list[Callback]
    betterlite.input_callback.append(_input_callback)
    betterlite.success_callback.append(_event_callback)
    assert _input_callback in betterlite.input_callback
    assert _event_callback in betterlite.success_callback
    betterlite.input_callback.pop()
    betterlite.success_callback.pop()


# ast-grep-ignore: no-long-names
def test_module_helper_call_rules_accept_typed_functions() -> None:
    hints: dict[str, type] = get_type_hints(settings_callbacks)
    for rules in (betterlite.pre_call_rules, betterlite.post_call_rules):
        rules.append(_call_rule)
        assert _call_rule in rules
        rules.pop()
    assert hints["pre_call_rules"] == list[Rule]
    assert hints["post_call_rules"] == list[Rule]


# ast-grep-ignore: no-long-names
def test_module_helper_thread_context_starts_with_hello_world() -> None:
    assert _thread_context_user() == "Hello World"
    assert betterlite.MyLocal().user == "Hello World"


# ast-grep-ignore: no-long-names
def test_module_helper_identify_keeps_the_user_per_thread() -> None:
    betterlite.identify({"user": "alice"})
    assert _thread_context_user() == "alice"
    betterlite.identify({})
    betterlite.identify({"event": "other"})
    assert _thread_context_user() == "alice"
    pool = ThreadPoolExecutor(max_workers=1)
    with pool:
        assert pool.submit(_identify_bob_in_this_thread).result() == "bob"
    assert _thread_context_user() == "alice"


# ast-grep-ignore: no-long-names
def test_module_helper_config_setters_change_the_globals() -> None:
    betterlite.set_global_bitbucket_config({"username": "someone"})
    betterlite.set_global_gitlab_config({"server_url": "https://gitlab.test"})
    members = _members(betterlite)
    assert members["global_bitbucket_config"] == {"username": "someone"}
    assert members["global_gitlab_config"] == {"server_url": "https://gitlab.test"}
    betterlite.global_bitbucket_config = None
    betterlite.global_gitlab_config = None


# ast-grep-ignore: no-long-names
def test_module_helper_rebinding_a_setting_holds() -> None:
    original = betterlite.drop_params
    betterlite.drop_params = True
    assert _members(betterlite)["drop_params"] is True
    betterlite.drop_params = original


# ast-grep-ignore: no-long-names
def test_module_helper_modify_integration_sets_the_supabase_table() -> None:
    betterlite.modify_integration("supabase", {"table_name": "my_logs"})
    assert betterlite.callback_settings.get("supabase") == {"table_name": "my_logs"}
    betterlite.callback_settings.pop("supabase", None)


# ast-grep-ignore: no-long-names
def test_module_helper_modify_integration_ignores_other_names() -> None:
    before = dict(betterlite.callback_settings)
    betterlite.modify_integration("langfuse", {"table_name": "ignored"})
    betterlite.modify_integration("supabase", {})
    assert betterlite.callback_settings == before
