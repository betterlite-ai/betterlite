from datetime import date
from enum import StrEnum
from typing import TypedDict

from pydantic import Field

from betterlite.types.utils import JsonValue, ResponseObject


class GroupByDimension(StrEnum):
    DATE = "date"
    MODEL = "model"
    API_KEY = "api_key"
    TEAM = "team"
    ORGANIZATION = "organization"
    MODEL_GROUP = "model_group"
    PROVIDER = "custom_llm_provider"


class SpendMetrics(ResponseObject):
    spend: float = 0.0
    flat_cost: float = 0.0
    prompt_tokens: int = 0
    completion_tokens: int = 0
    cache_read_input_tokens: int = 0
    cache_creation_input_tokens: int = 0
    compression_saved_tokens: int = 0
    compression_savings_spend: float = 0.0
    prompt_caching_savings_spend: float = 0.0
    # ast-grep-ignore: no-long-names
    gateway_injected_caching_savings_spend: float = 0.0
    autorouter_savings_spend: float = 0.0
    total_tokens: int = 0
    successful_requests: int = 0
    failed_requests: int = 0
    api_requests: int = 0
    total_response_time_ms: int = 0
    timed_requests: int = 0


class MetricBase(ResponseObject):
    metrics: SpendMetrics


class KeyMetadata(ResponseObject):
    key_alias: str | None = None
    team_id: str | None = None
    user_id: str | None = None
    user_email: str | None = None
    key_exists: bool | None = None


class KeyMetricWithMetadata(MetricBase):
    metadata: KeyMetadata = Field(default_factory=KeyMetadata)


class MetricWithMetadata(MetricBase):
    metadata: dict[str, JsonValue] = Field(default_factory=dict)
    api_key_breakdown: dict[str, KeyMetricWithMetadata] = Field(default_factory=dict)


class BreakdownMetrics(ResponseObject):
    mcp_servers: dict[str, MetricWithMetadata] = Field(default_factory=dict)
    models: dict[str, MetricWithMetadata] = Field(default_factory=dict)
    model_groups: dict[str, MetricWithMetadata] = Field(default_factory=dict)
    providers: dict[str, MetricWithMetadata] = Field(default_factory=dict)
    endpoints: dict[str, MetricWithMetadata] = Field(default_factory=dict)
    api_keys: dict[str, KeyMetricWithMetadata] = Field(default_factory=dict)
    entities: dict[str, MetricWithMetadata] = Field(default_factory=dict)


class DailySpendData(ResponseObject):
    date: date
    metrics: SpendMetrics
    breakdown: BreakdownMetrics = Field(default_factory=BreakdownMetrics)


class DailySpendMetadata(ResponseObject):
    total_spend: float = 0.0
    total_flat_cost: float = 0.0
    total_prompt_tokens: int = 0
    total_completion_tokens: int = 0
    total_tokens: int = 0
    total_api_requests: int = 0
    total_successful_requests: int = 0
    total_failed_requests: int = 0
    total_cache_read_input_tokens: int = 0
    # ast-grep-ignore: no-long-names
    total_cache_creation_input_tokens: int = 0
    total_compression_saved_tokens: int = 0
    # ast-grep-ignore: no-long-names
    total_compression_savings_spend: float = 0.0
    # ast-grep-ignore: no-long-names
    total_prompt_caching_savings_spend: float = 0.0
    # ast-grep-ignore: no-long-names
    total_gateway_injected_caching_savings_spend: float = 0.0
    total_autorouter_savings_spend: float = 0.0
    total_response_time_ms: int = 0
    total_timed_requests: int = 0
    page: int = 1
    total_pages: int = 1
    has_more: bool = False
    api_key_limit: int | None = None
    total_api_keys: int | None = None
    entity_total_api_keys: dict[str, int] | None = None


# ast-grep-ignore: no-long-names
class SpendAnalyticsPaginatedResponse(ResponseObject):
    results: list[DailySpendData]
    metadata: DailySpendMetadata = Field(default_factory=DailySpendMetadata)


class KeyActivityRow(ResponseObject):
    api_key: str
    metrics: SpendMetrics
    metadata: KeyMetadata


class KeySpendMetrics(ResponseObject):
    spend: float = 0.0
    prompt_tokens: int = 0
    completion_tokens: int = 0
    total_tokens: int = 0
    api_requests: int = 0
    successful_requests: int = 0
    failed_requests: int = 0
    cache_read_input_tokens: int = 0
    cache_creation_input_tokens: int = 0


class KeySpendActivityRow(ResponseObject):
    api_key: str
    metrics: KeySpendMetrics
    metadata: KeyMetadata


class DailyActivityKeySearchResponse(ResponseObject):
    api_keys: list[KeyActivityRow]


class DailyActivityKeyPageResponse(ResponseObject):
    api_keys: list[KeySpendActivityRow]
    total_api_keys: int
    offset: int
    limit: int


class ModelTopKeysResponse(ResponseObject):
    model: str
    by_model_group: bool
    api_keys: list[KeySpendActivityRow]


class CacheLeakageKeysResponse(ResponseObject):
    api_keys: list[KeySpendActivityRow]


class DailyUserSpend(ResponseObject):
    id: str
    user_id: str
    date: str
    api_key: str
    mcp_server_id: str | None = None
    model: str | None = None
    model_group: str | None = None
    custom_llm_provider: str | None = None
    prompt_tokens: int = 0
    completion_tokens: int = 0
    cache_read_input_tokens: int = 0
    cache_creation_input_tokens: int = 0
    compression_saved_tokens: int = 0
    compression_savings_spend: float = 0.0
    prompt_caching_savings_spend: float = 0.0
    # ast-grep-ignore: no-long-names
    gateway_injected_caching_savings_spend: float = 0.0
    autorouter_savings_spend: float = 0.0
    spend: float = 0.0
    api_requests: int = 0
    successful_requests: int = 0
    failed_requests: int = 0
    total_response_time_ms: int = 0
    timed_requests: int = 0


LiteLLM_DailyUserSpend = DailyUserSpend


class GroupedData(TypedDict):
    metrics: SpendMetrics
    breakdown: BreakdownMetrics
