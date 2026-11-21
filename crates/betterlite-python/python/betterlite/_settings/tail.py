from pydantic import JsonValue

import betterlite
from betterlite._settings.callbacks import callback_settings
from betterlite._settings.vertex_config import (
    VertexAITextEmbeddingConfig as vertexAITextEmbeddingConfig,
)

_key_management_settings: JsonValue = None
# ast-grep-ignore: no-long-names
_async_client_cleanup_registered = False
adapters: list[JsonValue] = []
vector_store_registry = None
vector_store_index_registry = None
vector_store_search_failure_mode = "annotate"  # ast-grep-ignore: no-long-names
custom_provider_map: list[JsonValue] = []
_custom_providers: list[str] = []
disable_hf_tokenizer_download = None
global_disable_no_log_param = False
global_bitbucket_config: dict[str, JsonValue] | None = None
global_gitlab_config: dict[str, JsonValue] | None = None


# ast-grep-ignore: no-one-line-functions
def set_global_bitbucket_config(config: dict[str, JsonValue]) -> None:
    betterlite.global_bitbucket_config = config


# ast-grep-ignore: no-one-line-functions
def set_global_gitlab_config(config: dict[str, JsonValue]) -> None:
    betterlite.global_gitlab_config = config


def modify_integration(
    integration_name: str,
    integration_params: dict[str, JsonValue],
) -> None:
    if integration_name != "supabase":
        return
    if "table_name" not in integration_params:
        return
    callback_settings.setdefault("supabase", {})["table_name"] = integration_params["table_name"]


__all__ = [
    "_async_client_cleanup_registered", "_custom_providers", "_key_management_settings", "adapters",
    "custom_provider_map", "disable_hf_tokenizer_download", "global_bitbucket_config",
    "global_disable_no_log_param", "global_gitlab_config", "modify_integration",
    "set_global_bitbucket_config", "set_global_gitlab_config", "vector_store_index_registry",
    "vector_store_registry", "vector_store_search_failure_mode", "vertexAITextEmbeddingConfig",
]
