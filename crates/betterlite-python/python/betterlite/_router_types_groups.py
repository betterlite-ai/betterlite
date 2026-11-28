from enum import Enum
from typing import ClassVar, Literal

from pydantic import ConfigDict, Field, JsonValue, model_validator
from typing_extensions import Required, TypedDict

from betterlite._router_types_config import CONFIGURABLE_CLIENTSIDE_AUTH_PARAMS
from betterlite._router_types_litellm_params import LiteLLMParamsTypedDict
from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.search import SearchProvider


class RouterErrors(Enum):
    user_defined_ratelimit_error = "Deployment over user-defined ratelimit."
    max_parallel_requests_exceeded = "Deployment has all max_parallel_requests slots in use."
    no_deployments_available = "No deployments available for selected model"
    all_deployments_in_cooldown = "All deployments for selected model are in cooldown"
    # ast-grep-ignore: no-long-names
    no_deployments_with_tag_routing = "Not allowed to access model due to tags configuration"
    # ast-grep-ignore: no-long-names
    no_deployments_with_provider_budget_routing = "No deployments available - crossed budget"
    no_healthy_deployments = "There are no healthy deployments for this model"
    only_strategy_marker_deployments = (  # ast-grep-ignore: no-long-names
        "Every deployment for it is a strategy router marker (auto_router/...), which is not a "
        "callable model, and no pre-routing strategy selected a deployment for this request"
    )


class AllowedFailsPolicy(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    BadRequestErrorAllowedFails: int | None = None
    AuthenticationErrorAllowedFails: int | None = None  # ast-grep-ignore: no-long-names
    TimeoutErrorAllowedFails: int | None = None
    RateLimitErrorAllowedFails: int | None = None
    ContentPolicyViolationErrorAllowedFails: int | None = None  # ast-grep-ignore: no-long-names
    InternalServerErrorAllowedFails: int | None = None  # ast-grep-ignore: no-long-names
    ServiceUnavailableErrorAllowedFails: int | None = None  # ast-grep-ignore: no-long-names
    BadGatewayErrorAllowedFails: int | None = None
    NotFoundErrorAllowedFails: int | None = None


class AlertingConfig(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    webhook_url: str
    alerting_threshold: float | None = 300


_ModelMode = Literal[
    "chat", "embedding", "completion", "image_generation", "audio_transcription", "rerank",
    "moderations",
]


class ModelGroupInfo(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    model_group: str
    providers: list[str]
    max_input_tokens: float | None = None
    max_output_tokens: float | None = None
    input_cost_per_token: float | None = None
    output_cost_per_token: float | None = None
    input_cost_per_pixel: float | None = None
    mode: str | _ModelMode | None = Field(default="chat")
    tpm: int | None = None
    rpm: int | None = None
    itpm: int | None = None
    otpm: int | None = None
    # ast-grep-ignore: no-long-names
    supports_parallel_function_calling: bool = Field(default=False)
    supports_vision: bool = Field(default=False)
    supports_web_search: bool = Field(default=False)
    supports_url_context: bool = Field(default=False)
    supports_reasoning: bool = Field(default=False)
    supports_function_calling: bool = Field(default=False)
    supports_fast_mode: bool = Field(default=False)
    supported_reasoning_efforts: tuple[str, ...] | None = Field(default=None)
    supported_openai_params: list[str] | None = Field(default=[])
    # ast-grep-ignore: no-long-names
    configurable_clientside_auth_params: CONFIGURABLE_CLIENTSIDE_AUTH_PARAMS = None

    @model_validator(mode="before")
    @classmethod
    def _unset_bool_is_false(cls, data: JsonValue) -> JsonValue:
        match data:
            case dict():
                filled: dict[str, JsonValue] = dict(data)
                for name in (
                    "supports_parallel_function_calling", "supports_vision",
                    "supports_web_search", "supports_url_context", "supports_reasoning",
                    "supports_function_calling", "supports_fast_mode",
                ):
                    if filled.get(name) is None:
                        filled[name] = False
                return filled
            case bool() | int() | float() | str() | list() | None:
                return data


class AssistantsTypedDict(TypedDict):
    custom_llm_provider: Literal["azure", "openai"]
    litellm_params: LiteLLMParamsTypedDict


class SearchToolLiteLLMParams(TypedDict, total=False):
    search_provider: Required[SearchProvider]
    api_key: str | None
    api_base: str | None
    timeout: float | str | None
    max_retries: int | None


class SearchToolInfoTypedDict(TypedDict, total=False):
    description: str


class SearchToolTypedDict(TypedDict, total=False):
    search_tool_name: Required[str]
    litellm_params: Required[SearchToolLiteLLMParams]
    search_tool_info: SearchToolInfoTypedDict


class GuardrailLiteLLMParams(TypedDict, total=False):
    guardrail: Required[str]
    mode: Required[str]
    api_key: str | None
    api_base: str | None
    weight: int | None


class GuardrailTypedDict(TypedDict, total=False):
    guardrail_name: Required[str]
    litellm_params: Required[GuardrailLiteLLMParams]
    callback: JsonValue
    id: str | None


class FineTuningConfig(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    custom_llm_provider: Literal["azure", "openai"]
