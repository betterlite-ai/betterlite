from typing import Annotated, ClassVar, Final, Literal

from pydantic import ConfigDict, Field, JsonValue, model_validator
from typing_extensions import Self, TypedDict

from betterlite.types.completion import CompletionRequest
from betterlite.types.embedding import EmbeddingRequest
from betterlite.types.litellm_params import RoutingStrategyName
from betterlite.types.router_weights import RouterWeights
from betterlite.types.utils.base import ResponseObject


class ConfigurableClientsideParamsCustomAuth(TypedDict):  # ast-grep-ignore: no-long-names
    api_base: str


# ast-grep-ignore: no-long-names
CONFIGURABLE_CLIENTSIDE_AUTH_PARAMS = list[str | ConfigurableClientsideParamsCustomAuth] | None


class ModelConfig(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", protected_namespaces=())

    model_name: str
    litellm_params: CompletionRequest | EmbeddingRequest
    tpm: int
    rpm: int


class RoutingGroup(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", protected_namespaces=())

    group_name: str
    models: list[str]
    routing_strategy: str
    routing_strategy_args: dict[str, JsonValue] | None = None
    model_priorities: (
        dict[str, Annotated[int, Field(strict=True, ge=1, le=9007199254740991)]] | None
    ) = Field(
        default=None, description=(
            "For priority groups, every model's priority. Lower numbers are tried first; "
            "equal numbers share traffic."
        ),
    )

    @model_validator(mode="after")
    def _validate_model_priorities(self) -> Self:
        if self.routing_strategy != "priority":
            if self.model_priorities is not None and len(self.model_priorities) > 0:
                raise ValueError("model_priorities requires routing_strategy='priority'")
            return self
        if len(self.models) == 0 or len(self.models) != len(frozenset(self.models)):
            raise ValueError("Priority routing groups require nonempty, distinct models")
        priorities: Final = self.model_priorities
        if priorities is None or frozenset(priorities) != frozenset(self.models):
            raise ValueError("model_priorities must contain exactly the group's models")
        strategy_args: Final = self.routing_strategy_args
        if strategy_args is not None and len(strategy_args) > 0:
            raise ValueError(
                "Priority routing groups use model_priorities, not routing_strategy_args"
            )
        return self


class RouterConfig(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", protected_namespaces=())

    model_list: list[ModelConfig]

    redis_url: str | None = None
    redis_host: str | None = None
    redis_port: int | None = None
    redis_password: str | None = None

    cache_responses: bool | None = False
    cache_kwargs: dict[str, JsonValue] | None = Field(default={})
    caching_groups: list[tuple[str, list[str]]] | None = None
    client_ttl: int | None = 3600
    num_retries: int | None = 0
    timeout: float | None = None
    default_litellm_params: dict[str, str] | None = Field(default={})
    set_verbose: bool | None = False
    fallbacks: list[JsonValue] | None = Field(default=[])
    allowed_fails: int | None = None
    context_window_fallbacks: list[JsonValue] | None = Field(default=[])
    model_group_alias: dict[str, list[str]] | None = Field(default={})
    retry_after: int | None = 0
    routing_strategy: RoutingStrategyName = "simple-shuffle"
    routing_groups: list[RoutingGroup] | None = None


class RetryPolicy(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    BadRequestErrorRetries: int | None = None
    AuthenticationErrorRetries: int | None = None
    TimeoutErrorRetries: int | None = None
    RateLimitErrorRetries: int | None = None
    ContentPolicyViolationErrorRetries: int | None = None  # ast-grep-ignore: no-long-names
    InternalServerErrorRetries: int | None = None
    ServiceUnavailableErrorRetries: int | None = None
    NotFoundErrorRetries: int | None = None
    DefaultRetries: int | None = None


OptionalPreCallChecks = list[Literal[
    "prompt_caching", "router_budget_limiting", "responses_api_deployment_check",
    "deployment_affinity", "session_affinity", "forward_client_headers_by_model_group",
    "enforce_model_rate_limits", "encrypted_content_affinity",
]]


class UpdateRouterConfig(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", protected_namespaces=())

    routing_strategy_args: dict[str, JsonValue] | None = None
    routing_strategy: str | None = None
    routing_groups: list[RoutingGroup] | None = None
    retry_policy: RetryPolicy | None = None
    model_group_retry_policy: dict[str, RetryPolicy] | None = None
    model_group_affinity_config: dict[str, list[str]] | None = None
    allowed_fails: int | None = None
    cooldown_time: float | None = None
    num_retries: int | None = None
    timeout: float | None = None
    max_retries: int | None = None
    retry_after: float | None = None
    fallbacks: list[dict[str, JsonValue]] | None = None
    context_window_fallbacks: list[dict[str, JsonValue]] | None = None
    model_group_alias: dict[str, str | dict[str, JsonValue]] | None = Field(default={})
    enable_tag_filtering: bool | None = None
    weights: RouterWeights | None = None
    tag_routing_prefix: str | None = None
    optional_pre_call_checks: OptionalPreCallChecks | None = None
