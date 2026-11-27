from collections.abc import Mapping
from dataclasses import dataclass
from typing import ClassVar, Final

from pydantic import ConfigDict, JsonValue
from typing_extensions import ReadOnly, Required, TypedDict, Unpack, override

from betterlite._router_types_config import CONFIGURABLE_CLIENTSIDE_AUTH_PARAMS
from betterlite._router_types_model_info import ModelInfo
from betterlite._router_types_params import LiteLLMParams, UpdateLiteLLMParams
from betterlite.types.utils.base import DumpOptions, InitValue, ResponseObject
from betterlite.types.utils.pricing import MirroredPricingParams
from betterlite.types.utils.response import ModelResponse


class LiteLLMParamsTypedDict(TypedDict, total=False):
    model: str
    custom_llm_provider: str | None
    tpm: int | None
    rpm: int | None
    itpm: int | None
    otpm: int | None
    order: int | None
    weight: int | None
    max_parallel_requests: int | None
    api_key: str | None
    api_base: str | None
    api_version: str | None
    timeout: float | str | None
    stream_timeout: float | str | None
    max_retries: int | None
    organization: list[JsonValue] | str | None
    provider_affinity_header: ReadOnly[str | None]
    # ast-grep-ignore: no-long-names
    configurable_clientside_auth_params: CONFIGURABLE_CLIENTSIDE_AUTH_PARAMS
    drop_params: bool | None
    use_chat_completions_api: bool | None
    use_in_pass_through: bool | None
    litellm_credential_name: str | None
    region_name: str | None
    vertex_project: str | None
    vertex_location: str | None
    aws_access_key_id: str | None
    aws_secret_access_key: str | None
    aws_region_name: str | None
    aws_bedrock_project_id: str | None
    vector_bucket_name: str | None
    index_name: str | None
    embedding_model: str | None
    watsonx_region_name: str | None
    input_cost_per_token: float | None
    output_cost_per_token: float | None
    cost_per_second: ReadOnly[float | None]
    input_cost_per_second: float | None
    output_cost_per_second: float | None
    output_cost_per_second_480p: ReadOnly[float | None]
    output_cost_per_second_720p: ReadOnly[float | None]
    output_cost_per_second_768p: ReadOnly[float | None]
    output_cost_per_second_2k: ReadOnly[float | None]
    output_cost_per_second_1080p: float | None
    output_cost_per_second_4k: ReadOnly[float | None]
    num_retries: int | None
    mock_response: str | ModelResponse | Exception | None

    tags: list[str] | None
    tag_regex: list[str] | None

    max_budget: float | None
    budget_duration: str | None
    keepalive_seconds: float | None
    allow_client_keepalive_override: bool | None  # ast-grep-ignore: no-long-names

    cooldown_time: float | None


class DeploymentTypedDict(TypedDict, total=False):
    model_name: Required[str]
    litellm_params: Required[LiteLLMParamsTypedDict]
    model_info: dict[str, JsonValue]


SPECIAL_MODEL_INFO_PARAMS: Final[tuple[str, ...]] = tuple(MirroredPricingParams.model_fields)


def _resolved_model_info(value: ModelInfo | dict[str, JsonValue] | None) -> ModelInfo:
    match value:
        case dict():
            return ModelInfo.model_validate(value)
        case ModelInfo() | None:
            return ModelInfo() if value is None else value


def _mirror_pricing(
    model_info: ModelInfo, litellm_params: LiteLLMParams | Mapping[str, JsonValue]
) -> None:
    match litellm_params:
        case LiteLLMParams():
            pricing: Final[dict[str, JsonValue]] = litellm_params.model_dump()
            for key in SPECIAL_MODEL_INFO_PARAMS:
                value = pricing.get(key)
                if value is not None:
                    model_info[key] = value
        case Mapping():
            pass


class Deployment(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", protected_namespaces=())

    model_name: str
    litellm_params: LiteLLMParams
    model_info: ModelInfo

    @override
    def __init__(
        self, model_name: str, litellm_params: LiteLLMParams | Mapping[str, JsonValue],
        model_info: ModelInfo | dict[str, JsonValue] | None = None, **params: InitValue,
    ) -> None:
        resolved: Final = _resolved_model_info(model_info)
        _mirror_pricing(resolved, litellm_params)
        super().__init__(
            model_info=resolved,
            model_name=model_name,
            litellm_params=litellm_params,
            **params,
        )

    def to_json(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        return self.model_dump(**kwargs)


@dataclass(frozen=True, slots=True)
class DiscoveredDeploymentModelInfo:
    deployment: Mapping[str, JsonValue]
    limits: Mapping[str, int]


@dataclass(frozen=True, slots=True)
class DeploymentModelListingInfo:
    cost_map_keys: tuple[str, ...]
    max_input_tokens: int | None
    max_output_tokens: int | None


class UpdateDeployment(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", protected_namespaces=())

    model_name: str | None = None
    litellm_params: UpdateLiteLLMParams | None = None
    model_info: ModelInfo | None = None
    blocked: bool | None = None
