from collections.abc import Sequence
from typing import Final

from pydantic import JsonValue, TypeAdapter

import betterlite

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


# Each setting a family may read, taken from betterlite at call time so a later assignment is seen
def module_settings() -> dict[str, JsonValue]:
    return {
        "add_function_to_prompt": JSON.validate_python(betterlite.add_function_to_prompt),
        "allowed_fails": JSON.validate_python(betterlite.allowed_fails),
        "api_base": JSON.validate_python(betterlite.api_base),
        "api_key": JSON.validate_python(betterlite.api_key),
        "api_version": JSON.validate_python(betterlite.api_version),
        "azure_key": JSON.validate_python(betterlite.azure_key),
        "cohere_key": JSON.validate_python(betterlite.cohere_key),
        "content_policy_fallbacks": JSON.validate_python(betterlite.content_policy_fallbacks),
        "context_window_fallbacks": JSON.validate_python(betterlite.context_window_fallbacks),
        "databricks_key": JSON.validate_python(betterlite.databricks_key),
        "default_fallbacks": JSON.validate_python(betterlite.default_fallbacks),
        "disable_stop_sequence_limit": JSON.validate_python(betterlite.disable_stop_sequence_limit),
        "drop_params": JSON.validate_python(betterlite.drop_params),
        "enable_json_schema_validation": JSON.validate_python(
            betterlite.enable_json_schema_validation
        ),
        "fallbacks": JSON.validate_python(betterlite.fallbacks),
        "groq_key": JSON.validate_python(betterlite.groq_key),
        "headers": JSON.validate_python(betterlite.headers),
        "huggingface_key": JSON.validate_python(betterlite.huggingface_key),
        "include_cost_in_streaming_usage": JSON.validate_python(
            betterlite.include_cost_in_streaming_usage
        ),
        "logging": JSON.validate_python(betterlite.logging),
        "max_fallbacks": JSON.validate_python(betterlite.max_fallbacks),
        "max_tokens": JSON.validate_python(betterlite.max_tokens),
        "model_alias_map": JSON.validate_python(betterlite.model_alias_map),
        "model_fallbacks": JSON.validate_python(betterlite.model_fallbacks),
        "modify_params": JSON.validate_python(betterlite.modify_params),
        "num_retries": JSON.validate_python(betterlite.num_retries),
        "num_retries_per_request": JSON.validate_python(betterlite.num_retries_per_request),
        "openai_key": JSON.validate_python(betterlite.openai_key),
        "openai_like_key": JSON.validate_python(betterlite.openai_like_key),
        "openrouter_key": JSON.validate_python(betterlite.openrouter_key),
        "organization": JSON.validate_python(betterlite.organization),
        "project": JSON.validate_python(betterlite.project),
        "return_response_headers": JSON.validate_python(betterlite.return_response_headers),
        "route_all_chat_openai_to_responses": JSON.validate_python(
            betterlite.route_all_chat_openai_to_responses
        ),
        "ssl_verify": JSON.validate_python(betterlite.ssl_verify),
        "togetherai_api_key": JSON.validate_python(betterlite.togetherai_api_key),
        "turn_off_message_logging": JSON.validate_python(betterlite.turn_off_message_logging),
        "use_client": JSON.validate_python(betterlite.use_client),
        "vertex_location": JSON.validate_python(betterlite.vertex_location),
        "vertex_project": JSON.validate_python(betterlite.vertex_project),
    }


def read_settings(names: Sequence[str]) -> dict[str, JsonValue]:
    available = module_settings()
    missing = [name for name in names if name not in available]
    if missing:
        raise AttributeError(f"betterlite has no setting named {missing[0]!r}")
    return {name: available[name] for name in names}
