from betterlite._core_helpers.errors import is_expected_client_error
from betterlite._core_helpers.filtering import (
    filter_exceptions_from_params,
    filter_internal_params,
)
from betterlite._core_helpers.flags import (
    coerce_token_limit,
    drop_params_env_flag,
    drop_params_flag,
    is_codex_user_agent,
    normalize_drop_params,
    safe_divide,
    safe_divide_seconds,
)
from betterlite._core_helpers.headers import (
    get_provider_response_headers_from_hidden_params,
    process_response_headers,
    set_provider_response_headers_in_hidden_params,
    set_response_cost_in_hidden_params,
)
from betterlite._core_helpers.messages import remove_index_from_tool_calls, remove_items_at_indices
from betterlite._core_helpers.metadata import (
    add_missing_spend_metadata_to_litellm_metadata,
    get_litellm_metadata_from_kwargs,
    get_metadata_variable_name_from_kwargs,
    get_or_create_metadata_bucket,
    max_retries_per_request_hit,
    proxy_stamped_used_client_oauth_token,
    reconstruct_model_name,
)
from betterlite._core_helpers.responses import preserve_upstream_non_openai_attributes
from betterlite._core_helpers.snapshots import independent_snapshot, safe_deep_copy
from betterlite.types.utils.choices import map_finish_reason

__all__ = [
    "add_missing_spend_metadata_to_litellm_metadata",
    "coerce_token_limit",
    "drop_params_env_flag",
    "drop_params_flag",
    "filter_exceptions_from_params",
    "filter_internal_params",
    "get_litellm_metadata_from_kwargs",
    "get_metadata_variable_name_from_kwargs",
    "get_or_create_metadata_bucket",
    "get_provider_response_headers_from_hidden_params",
    "independent_snapshot",
    "is_codex_user_agent",
    "is_expected_client_error",
    "map_finish_reason",
    "max_retries_per_request_hit",
    "normalize_drop_params",
    "preserve_upstream_non_openai_attributes",
    "process_response_headers",
    "proxy_stamped_used_client_oauth_token",
    "reconstruct_model_name",
    "remove_index_from_tool_calls",
    "remove_items_at_indices",
    "safe_deep_copy",
    "safe_divide",
    "safe_divide_seconds",
    "set_provider_response_headers_in_hidden_params",
    "set_response_cost_in_hidden_params",
]
