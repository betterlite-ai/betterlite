import asyncio
from typing import Literal

from litellm.router_strategy.adaptive_router.adaptive_router import AdaptiveRouter
from litellm.router_strategy.adaptive_router.classifier import classify_prompt
from litellm.types.router import (
    AdaptiveRouterConfig,
    AdaptiveRouterPreferences,
    RequestType,
)
from pydantic import JsonValue, ValidationError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

Check = Literal["classify", "minimum_quality", "router_configs", "preferences"]


class WeightsInput(Strict):
    quality: str | None = None
    cost: str | None = None


class RouterConfigInput(Strict):
    available_models: list[str]
    weights: WeightsInput | None = None


class PreferencesInput(Strict):
    quality_tier: int
    strengths: list[str] | None = None


class DeploymentInput(Strict):
    model_name: str
    adaptive_router_config: RouterConfigInput | None = None
    adaptive_router_preferences: PreferencesInput | None = None


class RequestInput(Strict):
    prompt: str
    headers: JsonValue | None = None
    metadata: JsonValue | None = None


class RouterOutput(Strict):
    model_name: str
    available_models: list[str]
    quality: str
    cost: str


class PreferenceOutput(Strict):
    model_name: str
    quality_tier: int
    strengths: list[RequestType]


class Output(Strict):
    types: list[RequestType] | None = None
    tiers: list[int | None] | None = None
    routers: list[RouterOutput] | None = None
    preferences: list[PreferenceOutput] | None = None
    invalid: bool | None = None


def parse_float(text: str, name: str) -> float:
    try:
        return float(text)
    except ValueError as error:
        raise ProbeError("invalid_request", f"{name} is not a number: {text}") from error


def config_values(raw: RouterConfigInput) -> dict[str, object]:
    values: dict[str, object] = {"available_models": raw.available_models}
    if raw.weights is not None:
        weights: dict[str, object] = {}
        if raw.weights.quality is not None:
            weights["quality"] = parse_float(raw.weights.quality, "weights.quality")
        if raw.weights.cost is not None:
            weights["cost"] = parse_float(raw.weights.cost, "weights.cost")
        values["weights"] = weights
    return values


def preference_values(raw: PreferencesInput) -> dict[str, object]:
    values: dict[str, object] = {"quality_tier": raw.quality_tier}
    if raw.strengths is not None:
        values["strengths"] = raw.strengths
    return values


def parsed(deployments: list[DeploymentInput]) -> tuple[list[RouterOutput], list[PreferenceOutput]] | None:
    routers: list[RouterOutput] = []
    preferences: list[PreferenceOutput] = []
    for deployment in deployments:
        raw_config = deployment.adaptive_router_config
        if raw_config is not None:
            try:
                config = AdaptiveRouterConfig.model_validate(config_values(raw_config))
            except ValidationError:
                return None
            routers.append(
                RouterOutput(
                    model_name=deployment.model_name,
                    available_models=config.available_models,
                    quality=f"{config.weights.quality:.6f}",
                    cost=f"{config.weights.cost:.6f}",
                )
            )
        raw_preferences = deployment.adaptive_router_preferences
        if raw_preferences is not None:
            try:
                prefs = AdaptiveRouterPreferences.model_validate(preference_values(raw_preferences))
            except ValidationError:
                return None
            preferences.append(
                PreferenceOutput(
                    model_name=deployment.model_name,
                    quality_tier=prefs.quality_tier,
                    strengths=prefs.strengths,
                )
            )
    return (routers, preferences)


def classify_all(prompts: list[str]) -> Output:
    return Output(types=[classify_prompt(prompt) for prompt in prompts])


class RecordingRouter(AdaptiveRouter):
    recorded: list[tuple[RequestType, int | None]]

    def __init__(self, config: AdaptiveRouterConfig) -> None:
        super().__init__(router_name="probe", config=config, model_to_prefs={}, model_to_cost={})
        self.recorded = []

    @override
    async def pick_model(self, request_type: RequestType, min_quality_tier: int | None = None) -> str:
        self.recorded.append((request_type, min_quality_tier))
        return self.config.available_models[0]


class AdaptiveClassification(Operation):
    check: Check
    prompts: list[str]
    requests: list[RequestInput]
    deployments: list[DeploymentInput]

    @override
    def run(self) -> Output:
        match self.check:
            case "classify":
                return classify_all(self.prompts)
            case "minimum_quality":
                return asyncio.run(self.minimum_quality())
            case "router_configs":
                parsed_fields = parsed(self.deployments)
                if parsed_fields is None:
                    return Output(invalid=True)
                return Output(routers=parsed_fields[0])
            case "preferences":
                parsed_fields = parsed(self.deployments)
                if parsed_fields is None:
                    return Output(invalid=True)
                return Output(preferences=parsed_fields[1])

    async def minimum_quality(self) -> Output:
        router = RecordingRouter(AdaptiveRouterConfig(available_models=["fast"]))
        for request in self.requests:
            _ = await router.async_pre_routing_hook(
                model="probe",
                request_kwargs={"headers": request.headers, "metadata": request.metadata},
                messages=[{"role": "user", "content": request.prompt}],
            )
        return Output(
            types=[request_type for request_type, _tier in router.recorded],
            tiers=[tier for _request_type, tier in router.recorded],
        )
