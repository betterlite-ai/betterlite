from collections.abc import Mapping
from dataclasses import dataclass
from enum import Enum
from typing import ClassVar, Final, Generic, Protocol, TypeVar, runtime_checkable

from pydantic import ConfigDict, Field, JsonValue, ValidationInfo, field_validator
from typing_extensions import TypedDict

from betterlite.types.litellm_params_state import ForeignObject
from betterlite.types.llms.openai.core.files import OpenAIFileObject
from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.routing import StandardLoggingRoutingDecision


class FallbackAccessCheck(Protocol):
    async def __call__(
        self, *, model: str, request_kwargs: Mapping[str, JsonValue], llm_router: ForeignObject
    ) -> bool: ...


class FallbackBudgetCheck(Protocol):
    async def __call__(
        self, *, model: str, request_kwargs: Mapping[str, JsonValue], llm_router: ForeignObject
    ) -> bool: ...


class AutoRouterCapabilityLimit(Protocol):
    def __call__(self) -> int | None: ...


class LiteLLMRouterFileObject(TypedDict, total=False):
    litellm_params_sensitive_credential_hash: str  # ast-grep-ignore: no-long-names
    file_object: OpenAIFileObject


def _extract_bool_param(kwargs: dict[str, JsonValue], name: str) -> bool | None:
    value: Final = kwargs.pop(name, None)
    if value is None:
        return None
    match value:
        case bool():
            return value
        case str():
            flags: Final = {"true": True, "false": False}
            return flags.get(value.strip().lower())
        case int() | float() | list() | dict():
            return None


@dataclass
class MockRouterTestingParams:
    mock_testing_fallbacks: bool | None = None
    mock_testing_context_fallbacks: bool | None = None
    mock_testing_content_policy_fallbacks: bool | None = None  # ast-grep-ignore: no-long-names

    @classmethod
    def from_kwargs(cls, kwargs: dict[str, JsonValue]) -> "MockRouterTestingParams":
        fallbacks: Final = _extract_bool_param(kwargs, "mock_testing_fallbacks")
        context_fallbacks: Final = _extract_bool_param(kwargs, "mock_testing_context_fallbacks")
        content_policy_fallbacks: Final = _extract_bool_param(
            kwargs, "mock_testing_content_policy_fallbacks"
        )
        return cls(
            mock_testing_fallbacks=fallbacks, mock_testing_context_fallbacks=context_fallbacks,
            mock_testing_content_policy_fallbacks=content_policy_fallbacks,
        )


class ModelGroupSettings(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    forward_client_headers_to_llm_api: list[str] | None = None  # ast-grep-ignore: no-long-names


class PreRoutingHookResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    model: str
    messages: list[dict[str, JsonValue]] | None
    routing_decision: StandardLoggingRoutingDecision | None = None
    session_affinity_ttl_seconds: int | None = None
    litellm_params: Mapping[str, JsonValue] | None = None


_PreRoutingStrategyT_co = TypeVar("_PreRoutingStrategyT_co", covariant=True)


@dataclass(frozen=True, slots=True)
class TaggedPreRoutingStrategy(Generic[_PreRoutingStrategyT_co]):
    tags: tuple[str, ...]
    strategy: _PreRoutingStrategyT_co


@dataclass(frozen=True, slots=True)
class BaselineRouteStamp:
    router_name: str
    baseline_model: str
    baseline_deployment_id: str


@dataclass(frozen=True, slots=True)
class ConsumedRequestTagsStamp:
    model_group: str
    tags: tuple[str, ...]


@runtime_checkable
class PreRoutingStrategy(Protocol):
    async def async_pre_routing_hook(
        self, model: str, request_kwargs: dict[str, JsonValue],
        messages: list[dict[str, JsonValue]] | None = None,
        input_value: str | list[JsonValue] | None = None, *,
        specific_deployment: bool | None = False,
    ) -> PreRoutingHookResponse | None: ...


class RoutingContext(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    raw_messages: list[dict[str, JsonValue]]
    structured_messages: list[dict[str, JsonValue]]
    candidate_models: list[str]
    metadata: dict[str, JsonValue] = Field(default_factory=dict)
    signals: dict[str, JsonValue] = Field(default_factory=dict)


@runtime_checkable
class RoutingPlugin(Protocol):
    async def run(self, context: RoutingContext) -> RoutingContext: ...


@runtime_checkable
class ClassifierPlugin(Protocol):
    async def classify(self, context: RoutingContext) -> str | None: ...


class RequestType(str, Enum):
    CODE_GENERATION = "code_generation"
    CODE_UNDERSTANDING = "code_understanding"
    TECHNICAL_DESIGN = "technical_design"
    ANALYTICAL_REASONING = "analytical_reasoning"
    WRITING = "writing"
    FACTUAL_LOOKUP = "factual_lookup"
    GENERAL = "general"


_DEFAULT_QUALITY: Final = 0.7
_WEIGHT_SUM_TARGET: Final = 1.0
_WEIGHT_SUM_TOLERANCE: Final = 0.001


def _quality_value(value: JsonValue) -> float:
    match value:
        case float():
            return value
        case bool() | int():
            return float(value)
        case str() | list() | dict() | None:
            return _DEFAULT_QUALITY


class AdaptiveRouterWeights(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    quality: float = Field(default=_DEFAULT_QUALITY, ge=0.0, le=1.0)
    cost: float = Field(default=0.3, ge=0.0, le=1.0)

    @field_validator("cost")
    @classmethod
    def _weights_sum_to_one(cls, value: float, info: ValidationInfo[None]) -> float:
        data: Final[dict[str, JsonValue]] = info.data
        quality: Final = _quality_value(data.get("quality"))
        if abs(quality + value - _WEIGHT_SUM_TARGET) > _WEIGHT_SUM_TOLERANCE:
            raise ValueError(
                f"weights must sum to 1.0, got quality={quality} + cost={value} = {quality + value}"
            )
        return value


class AdaptiveRouterConfig(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    available_models: list[str]
    weights: AdaptiveRouterWeights = Field(default_factory=AdaptiveRouterWeights)


class AdaptiveRouterPreferences(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", use_enum_values=False)

    quality_tier: int = Field(ge=1, le=3)
    strengths: list[RequestType] = Field(default_factory=list)
