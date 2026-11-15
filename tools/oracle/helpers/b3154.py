import asyncio
from typing import Final, Literal, TypeAlias, TypedDict

import httpx
import litellm
from litellm.proxy.roi_calculator.estimator import (
    Estimator,
    cache_context,
    estimator_options,
    metadata_evidence,
)
from litellm.types.roi_calculator import (
    ROICompletionRequest,
    ROIEstimatorResult,
    ROIPullEvidence,
    ROISettings,
)
from pydantic import JsonValue, TypeAdapter

_JSON: Final = TypeAdapter(JsonValue)

ModelList: TypeAlias = list[tuple[str, str | None]]


class SettingsInput(TypedDict):
    estimator_model: str
    estimator_prompt: str


class MetadataEvidenceInput(TypedDict):
    operation: Literal["metadata_evidence"]
    pull: ROIPullEvidence


class EstimatorOptionsInput(TypedDict):
    operation: Literal["estimator_options"]
    models: ModelList
    supported: list[str]


class ConfiguredModelsInput(TypedDict):
    operation: Literal["configured_models"]
    settings: SettingsInput
    models: ModelList | None


class CacheContextInput(TypedDict):
    operation: Literal["cache_context"]
    settings: SettingsInput
    models: ModelList | None
    supported: list[str]


class EstimateInput(TypedDict):
    operation: Literal["estimate"]
    settings: SettingsInput
    models: ModelList | None
    supported: list[str]
    pull: ROIPullEvidence
    response: JsonValue
    response_is_error: bool


class ValidateResultInput(TypedDict):
    operation: Literal["validate_result"]
    raw: str


Input: TypeAlias = (
    MetadataEvidenceInput
    | EstimatorOptionsInput
    | ConfiguredModelsInput
    | CacheContextInput
    | EstimateInput
    | ValidateResultInput
)


class FakeCompletion:
    def __init__(self, response: JsonValue, *, is_error: bool) -> None:
        self.response: JsonValue = response
        self.is_error: bool = is_error
        self.requests: list[JsonValue] = []

    async def __call__(self, request: ROICompletionRequest) -> JsonValue:
        self.requests.append(_JSON.validate_json(request.model_dump_json()))
        if self.is_error:
            raise httpx.ConnectError("the scripted completion failed")
        return self.response


def settings_of(value: SettingsInput) -> ROISettings:
    return ROISettings(
        estimator_model=value["estimator_model"],
        estimator_prompt=value["estimator_prompt"],
    )


def install_model_cost(supported: list[str], models: ModelList) -> None:
    costs: dict[str, dict[str, str | bool]] = {}
    for model, provider in models:
        costs[model] = {
            "litellm_provider": "openai" if provider is None else provider,
            "mode": "chat",
            "supports_none_reasoning_effort": model in supported,
        }
    litellm.model_cost = costs


def configured_models(value: ModelList | None) -> tuple[tuple[str, str | None], ...] | None:
    if value is None:
        return None
    return tuple(value)


def evidence_value(value: MetadataEvidenceInput) -> JsonValue:
    evidence = metadata_evidence(value["pull"])
    return _JSON.validate_json(evidence.model_dump_json(exclude_unset=True))


def options_value(value: EstimatorOptionsInput) -> JsonValue:
    install_model_cost(value["supported"], value["models"])
    options = estimator_options(tuple(value["models"]))
    fields: dict[str, JsonValue] = {}
    if "reasoning_effort" in options:
        fields["reasoning_effort"] = options["reasoning_effort"]
    return fields


def configured_value(value: ConfiguredModelsInput) -> JsonValue:
    estimator = Estimator(
        settings_of(value["settings"]),
        FakeCompletion(None, is_error=False),
        configured_models(value["models"]),
    )
    pairs: list[JsonValue] = []
    for model, provider in estimator.models:
        pairs.append([model, provider])
    return pairs


def context_value(value: CacheContextInput) -> JsonValue:
    models = value["models"]
    if models is not None:
        install_model_cost(value["supported"], models)
    return cache_context(settings_of(value["settings"]), configured_models(models))


def estimate_value(value: EstimateInput) -> JsonValue:
    models = value["models"]
    if models is not None:
        install_model_cost(value["supported"], models)
    complete = FakeCompletion(
        value["response"], is_error=value["response_is_error"]
    )
    estimator = Estimator(
        settings_of(value["settings"]), complete, configured_models(models)
    )
    estimate = asyncio.run(estimator.estimate(value["pull"]))
    dumped = _JSON.validate_python(estimate)
    return {"estimate": dumped, "requests": complete.requests}


def result_value(value: ValidateResultInput) -> JsonValue:
    result = ROIEstimatorResult.model_validate_json(value["raw"])
    return {"hours": float(result.hours), "reasoning": result.reasoning}


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "metadata_evidence":
        return evidence_value(input_value)
    if input_value["operation"] == "estimator_options":
        return options_value(input_value)
    if input_value["operation"] == "configured_models":
        return configured_value(input_value)
    if input_value["operation"] == "cache_context":
        return context_value(input_value)
    if input_value["operation"] == "estimate":
        return estimate_value(input_value)
    return result_value(input_value)
