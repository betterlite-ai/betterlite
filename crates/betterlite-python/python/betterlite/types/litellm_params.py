from collections.abc import Callable, Mapping, MutableMapping, Sequence
from dataclasses import Field as DataclassField
from dataclasses import dataclass, field, fields
from itertools import chain
from typing import Annotated, ClassVar, Final, Literal, Protocol, get_type_hints

from pydantic import BeforeValidator, Field, JsonValue
from pydantic.dataclasses import dataclass as pydantic_dataclass

from betterlite.types.caching import DynamicCacheControl
from betterlite.types.litellm_params_state import (
    ADDRESSED_RESPONSE_ID_FIELD,
    TRUSTED_CALLBACK_VARS_FIELD,
    WIRE_NAME,
    AgenticLoopState,
    AgenticSurface,
    CallState,
    EntrypointState,
    ForeignObject,
    InternalState,
    JsonMap,
    ProxyRequestState,
    RouterState,
)
from betterlite.types.utils.response import ModelResponse, ModelResponseStream

__all__ = [
    "ADDRESSED_RESPONSE_ID_FIELD", "TRUSTED_CALLBACK_VARS_FIELD", "AgenticSurface", "CallState",
    "EntrypointState", "ProxyRequestState", "RouterState",
]

RetryStrategy = Literal["constant_retry", "exponential_backoff_retry"]
RoutingStrategyName = Literal[
    "simple-shuffle", "least-busy", "usage-based-routing", "latency-based-routing",
    "cost-based-routing", "usage-based-routing-v2", "lar1",
]
MockResponse = str | Exception | JsonMap | Sequence[float] | ModelResponse | ModelResponseStream


class DataclassType(Protocol):
    __dataclass_fields__: ClassVar[dict[str, DataclassField[JsonValue]]]


@dataclass(frozen=True, slots=True, kw_only=True)
class ProviderConnection:
    api_key: str | None = None
    api_base: str | None = None
    api_version: str | None = None
    region_name: str | None = None
    headers: Mapping[str, str] | None = None
    provider_specific_header: JsonMap | Sequence[JsonMap] | None = None
    client: ForeignObject | None = None
    shared_session: ForeignObject | None = None
    ssl_verify: bool | str | None = None
    request_timeout: float | None = None
    force_timeout: float | None = None
    stream_timeout: float | str | None = None
    max_retries: int | None = None
    tenant_id: str | None = None
    client_id: str | None = None
    client_secret: str | None = None
    azure_username: str | None = None
    azure_password: str | None = None
    azure_scope: str | None = None
    azure_ad_token_provider: Callable[[], str] | None = None
    litellm_credential_name: str | None = None
    # ast-grep-ignore: no-long-names
    configurable_clientside_auth_params: Sequence[str | JsonMap] | None = None
    use_xai_oauth: bool | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class BedrockBatchConnection:
    aws_batch_role_arn: str | None = None
    s3_bucket_name: str | None = None
    s3_region_name: str | None = None
    s3_endpoint_url: str | None = None
    s3_output_bucket_name: str | None = None
    s3_bucket_owner: str | None = None
    s3_access_key_id: str | None = None
    s3_secret_access_key: str | None = None
    s3_encryption_key_id: str | None = None
    bedrock_tags: Sequence[Mapping[str, str]] | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class ConnectionSettings:
    provider: ProviderConnection
    bedrock_batch: BedrockBatchConnection


@dataclass(frozen=True, slots=True, kw_only=True)
class DispatchOptions:
    custom_llm_provider: str | None = None
    azure: bool | None = None
    use_litellm_proxy: bool | None = None
    use_chat_completions_api: bool | None = None
    use_in_pass_through: bool | None = None
    allowed_openai_params: Sequence[str] | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class RoutingOptions:
    fallbacks: Sequence[str | JsonMap] | None = None
    context_window_fallback_dict: Mapping[str, str] | None = None
    num_retries: int | None = None
    retry_policy: JsonMap | None = None
    retry_strategy: RetryStrategy | None = None
    routing_strategy: RoutingStrategyName | None = None
    cooldown_time: float | None = None
    allowed_model_region: str | None = None
    enable_tag_filtering: bool | None = None
    fastest_response: bool | None = None
    provider_affinity_header: str | None = None
    search_tool_name: str | None = None
    model_list: Sequence[JsonMap] | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class DeploymentOptions:
    model_info: JsonMap | None = None
    rpm: int | None = None
    tpm: int | None = None
    itpm: int | None = None
    otpm: int | None = None
    default_api_key_rpm_limit: int | None = None
    default_api_key_tpm_limit: int | None = None
    max_parallel_requests: int | None = None
    weight: int | None = None
    order: int | None = None
    tag_regex: Sequence[str] | None = None
    max_file_size_mb: float | None = None
    silent_model: str | Sequence[str] | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class SpecializedRouterOptions:
    auto_router_config_path: str | None = None
    auto_router_config: str | None = None
    auto_router_default_model: str | None = None
    auto_router_embedding_model: str | None = None
    auto_router_max_input_chars: int | None = None
    auto_router_routing_compression: str | None = None  # ast-grep-ignore: no-long-names
    auto_router_model_compression: str | None = None
    complexity_router_config: JsonMap | None = None
    complexity_router_default_model: str | None = None  # ast-grep-ignore: no-long-names
    adaptive_router_config: JsonMap | None = None
    adaptive_router_default_model: str | None = None
    quality_router_config: JsonMap | None = None
    quality_router_default_model: str | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class CachingOptions:
    caching: bool | None = None
    cache: DynamicCacheControl | None = None
    ttl: float | None = None
    enable_prompt_caching: bool | None = None
    caching_groups: Sequence[Sequence[str]] | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class CostOptions:
    cost_per_query: float | None = None
    base_model: str | None = None
    max_budget: float | None = None
    budget_duration: str | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class ObservabilityOptions:
    id: str | None = None
    metadata: MutableMapping[str, JsonValue] | None = None
    litellm_metadata: MutableMapping[str, JsonValue] | None = None
    tags: Sequence[str] | None = None
    litellm_trace_id: str | None = None
    litellm_session_id: str | None = None
    litellm_request_debug: bool | None = None
    logger_fn: Callable[[JsonMap], None] | None = None
    verbose: bool | None = None
    no_log: bool | None = field(default=None, metadata={WIRE_NAME: "no-log"})
    log_client_error_tracebacks: bool | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class AgenticLoopOptions:
    max_agentic_loops: int | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class GuardrailOptions:
    guardrails: Sequence[str] | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class PromptOptions:
    prompt_id: str | None = None
    prompt_variables: JsonMap | None = None
    prompt_version: str | None = None
    prompt_environment: str | None = None
    prompt_label: str | None = None
    litellm_system_prompt: str | None = None
    custom_prompt_dict: JsonMap | None = None
    roles: JsonMap | None = None
    final_prompt_value: str | None = None
    bos_token: str | None = None
    eos_token: str | None = None
    hf_model_name: str | None = None
    supports_system_message: bool | None = None
    ensure_alternating_roles: bool | None = None
    user_continue_message: JsonMap | None = None
    assistant_continue_message: JsonMap | None = None
    disable_add_transform_inline_image_block: bool | None = None  # ast-grep-ignore: no-long-names


@dataclass(frozen=True, slots=True, kw_only=True)
class ResponseOptions:
    merge_reasoning_content_in_choices: bool | None = None  # ast-grep-ignore: no-long-names
    enable_json_schema_validation: bool | None = None
    complete_response: bool | None = None
    keepalive_seconds: float | None = None
    allow_client_keepalive_override: bool | None = None  # ast-grep-ignore: no-long-names


MAX_CONTROL_INT_DIGITS: Final = 18


def _int_from_decimal_string(value: JsonValue) -> JsonValue:
    match value:
        case str() if (
            value.isascii() and value.isdecimal() and len(value) <= MAX_CONTROL_INT_DIGITS
        ):
            return int(value)
        case str() | bool() | int() | float() | list() | dict() | None:
            return value


@pydantic_dataclass(frozen=True, slots=True, kw_only=True)
class ControlOptions:
    stream_chunk_size: (
        Annotated[
            int,
            BeforeValidator(_int_from_decimal_string),
            Field(strict=True, gt=0, lt=10**MAX_CONTROL_INT_DIGITS),
        ]
        | None
    ) = None


@dataclass(frozen=True, slots=True, kw_only=True)
class MockOptions:
    mock_response: MockResponse | None = None
    mock_timeout: bool | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class LiteLLMOptions:
    dispatch: DispatchOptions
    routing: RoutingOptions
    deployment: DeploymentOptions
    specialized_routers: SpecializedRouterOptions
    caching: CachingOptions
    cost: CostOptions
    observability: ObservabilityOptions
    agentic_loop: AgenticLoopOptions
    guardrails: GuardrailOptions
    prompt: PromptOptions
    response: ResponseOptions
    control: ControlOptions
    mock: MockOptions


KWARG_ARTIFACTS: Final[tuple[str, ...]] = ("self", "use_client", "model_config", "rust")

LITELLM_OWNED_ROOTS: Final[tuple[type[DataclassType], ...]] = (
    ConnectionSettings,
    LiteLLMOptions,
    InternalState,
)


def wire_names(owner: type[DataclassType]) -> tuple[str, ...]:
    names: list[str] = []
    for owned in fields(owner):
        metadata: Mapping[str, str] = owned.metadata
        names.append(metadata[WIRE_NAME] if WIRE_NAME in metadata else owned.name)
    return tuple(names)


def owned_wire_names(root: type[DataclassType]) -> tuple[str, ...]:
    leaves: dict[str, type[DataclassType]] = get_type_hints(root)
    return tuple(chain.from_iterable(wire_names(leaf) for leaf in leaves.values()))


OWNED_KWARG_NAMES: Final = tuple(
    chain.from_iterable(owned_wire_names(root) for root in LITELLM_OWNED_ROOTS)
)
AGENTIC_LOOP_KWARG_NAMES: Final = (*wire_names(AgenticLoopState), *wire_names(AgenticLoopOptions))
BEDROCK_BATCH_KWARG_NAMES: Final = wire_names(BedrockBatchConnection)
