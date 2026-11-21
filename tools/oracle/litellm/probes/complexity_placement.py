import asyncio
from collections.abc import Callable, Coroutine, Mapping, Sequence
from typing import Final, TypeAlias
from unittest.mock import patch

import litellm
from litellm.router import Router
from litellm.router_strategy.complexity_router import complexity_router
from litellm.router_strategy.complexity_router.complexity_router import ComplexityRouter
from pydantic import Field, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

from probes.protocol import InjectedRandom, Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true
# pyright: reportMissingTypeStubs=false, reportUnknownMemberType=false, reportUnusedCallResult=false

_PICK: Callable[..., Coroutine[object, object, str]] = TypeAdapter(
    Callable[..., Coroutine[object, object, str]]
).validate_python(ComplexityRouter.__dict__["_pick_model_for_tier"])

_PARAMS: Callable[..., Mapping[str, object]] = TypeAdapter(
    Callable[..., Mapping[str, object]]
).validate_python(ComplexityRouter.__dict__["_litellm_params_for_model"])

_TIER_POOLS: Callable[..., dict[str, list[str]]] = TypeAdapter(
    Callable[..., dict[str, list[str]]]
).validate_python(ComplexityRouter.__dict__["_tier_pools"])

_TIER_NAME: Callable[..., str] = TypeAdapter(Callable[..., str]).validate_python(
    complexity_router.__dict__["_tier_name"]
)

_PIN_TIER: Callable[..., bool] = TypeAdapter(Callable[..., bool]).validate_python(
    Router.__dict__["_pin_tier_params_onto_request"]
)

_RESTORE: Callable[..., None] = TypeAdapter(Callable[..., None]).validate_python(
    Router.__dict__["_restore_client_ceiling_no_tier_pins"]
)

_MAP: Final = TypeAdapter(dict[str, JsonValue])
RAW_NAME_KEY: Final = "_complexity_router_return_raw_model_name"


class TierEntryInput(Strict):
    model_name: str
    litellm_params: dict[str, JsonValue] = Field(default_factory=dict)


TierValue: TypeAlias = str | TierEntryInput | list[str | TierEntryInput]


class DeploymentInput(Strict):
    group: str
    model: str | None = None
    custom_llm_provider: str | None = None
    max_output_tokens: int | None = None
    allowed_openai_params: list[str] = Field(default_factory=list)


class StepInput(Strict):
    tier: str
    allowed_models: list[str] | None = None
    responses_call: bool = False
    request_kwargs: dict[str, JsonValue] = Field(default_factory=dict)


class StepOutput(Strict):
    model: str
    tier: str
    pinned: bool
    request_kwargs: dict[str, JsonValue]


class Output(Strict):
    tier_pools: dict[str, list[str]]
    steps: list[StepOutput]


# the world's get_model_list and get_router_model_info, answered from the probe's own deployments
class ProbeRouter:
    def __init__(self, deployments: list[DeploymentInput]) -> None:
        self.groups: dict[str, list[DeploymentInput]] = {}
        for deployment in deployments:
            self.groups.setdefault(deployment.group, []).append(deployment)

    def get_model_list(
        self, model_name: str, team_id: str | None = None
    ) -> list[dict[str, JsonValue]] | None:
        _ = team_id
        deployments = self.groups.get(model_name)
        if deployments is None:
            return None
        return [self._deployment(deployment) for deployment in deployments]

    def get_router_model_info(
        self, deployment: dict[str, JsonValue], received_model_name: str
    ) -> dict[str, JsonValue]:
        _ = received_model_name
        return _MAP.validate_python(deployment.get("model_info"))

    def _deployment(self, deployment: DeploymentInput) -> dict[str, JsonValue]:
        declared = "" if deployment.model is None else deployment.model
        params: dict[str, JsonValue] = {"model": declared}
        if deployment.custom_llm_provider is not None:
            params["custom_llm_provider"] = deployment.custom_llm_provider
        if deployment.allowed_openai_params:
            allowed: list[JsonValue] = list(deployment.allowed_openai_params)
            params["allowed_openai_params"] = allowed
        info: dict[str, JsonValue] = {}
        if deployment.max_output_tokens is not None:
            info["max_output_tokens"] = deployment.max_output_tokens
        return {"litellm_params": params, "model_info": info}


# get_supported_openai_params answers from the case, so both sides see one provider capability map
class SupportedParams:
    def __init__(self, supported: dict[str, list[str]]) -> None:
        self.supported: dict[str, list[str]] = supported

    def __call__(
        self, model: str, custom_llm_provider: str | None = None, base_model: str | None = None
    ) -> list[str] | None:
        _ = (custom_llm_provider, base_model)
        return self.supported.get(model)


# random.choice does not route through random(), so the pick consumes the injected sample itself
class InjectedChoice:
    def __init__(self, generator: InjectedRandom) -> None:
        self.generator: InjectedRandom = generator

    def choice(self, seq: Sequence[str]) -> str:
        picked = int(self.generator.random() * len(seq))
        return seq[picked]


class ComplexityPlacement(Operation):
    return_raw_model_name: bool = False
    max_tokens_from_tier_model: bool = True
    tiers: dict[str, TierValue] = Field(default_factory=dict)
    default_model: str | None = None
    deployment_pin: bool = False
    deployments: list[DeploymentInput] = Field(default_factory=list)
    supported_params: dict[str, list[str]] = Field(default_factory=dict)
    steps: list[StepInput] = Field(default_factory=list)

    @override
    def run_with(self, generator: InjectedRandom) -> Output:
        return asyncio.run(self._run(generator))

    async def _run(self, generator: InjectedRandom) -> Output:
        router = Router()
        probe_router = ProbeRouter(self.deployments)
        object.__setattr__(router, "get_model_list", probe_router.get_model_list)
        object.__setattr__(router, "get_router_model_info", probe_router.get_router_model_info)
        complexity = ComplexityRouter(
            model_name="probe",
            litellm_router_instance=router,
            complexity_router_config=self._configured(),
        )
        with (
            patch.object(complexity_router, "random", InjectedChoice(generator)),
            patch.object(
                litellm,
                "get_supported_openai_params",
                SupportedParams(self.supported_params),
            ),
        ):
            tier_pools = _TIER_POOLS(complexity)
            outputs = [await self._step(complexity, router, step) for step in self.steps]
            return Output(tier_pools=tier_pools, steps=outputs)

    async def _step(
        self, complexity: ComplexityRouter, router: Router, step: StepInput
    ) -> StepOutput:
        kwargs: dict[str, JsonValue] = dict(step.request_kwargs)
        self._mark_raw_model_name(kwargs)
        allowed: tuple[str, ...] | None = (
            None if step.allowed_models is None else tuple(step.allowed_models)
        )
        try:
            model = await _PICK(complexity, step.tier, None, None, kwargs, allowed, None)
            params = _PARAMS(complexity, step.tier, model)
            pinned = _PIN_TIER(router, model, params, kwargs, step.responses_call)
            if not pinned:
                _RESTORE(kwargs)
        except ValueError as error:
            raise ProbeError("invalid_request", str(error)) from error
        return StepOutput(
            model=model,
            tier=_TIER_NAME(step.tier),
            pinned=pinned,
            request_kwargs=_MAP.validate_python(kwargs),
        )

    # async_pre_routing_hook's first step: the response model field keeps the raw selected name
    def _mark_raw_model_name(self, kwargs: dict[str, JsonValue]) -> None:
        if not self.return_raw_model_name:
            return
        key: Final = "litellm_metadata" if "litellm_metadata" in kwargs else "metadata"
        if key not in kwargs:
            bucket: dict[str, JsonValue] = {RAW_NAME_KEY: True}
            kwargs[key] = bucket
            return
        try:
            stamped = _MAP.validate_python(kwargs[key])
        except ValidationError:
            return
        stamped[RAW_NAME_KEY] = True
        kwargs[key] = stamped

    def _configured(self) -> dict[str, object]:
        return {
            "tiers": {tier: self._tier_value(value) for tier, value in self.tiers.items()},
            "default_model": self.default_model,
            "deployment_affinity": self.deployment_pin,
            "max_tokens_from_tier_model": self.max_tokens_from_tier_model,
            "return_raw_model_name": self.return_raw_model_name,
        }

    def _tier_value(
        self, value: TierValue
    ) -> str | dict[str, JsonValue] | list[str | dict[str, JsonValue]]:
        if isinstance(value, str):
            return value
        if isinstance(value, TierEntryInput):
            return self._tier_entry(value)
        return [entry if isinstance(entry, str) else self._tier_entry(entry) for entry in value]

    def _tier_entry(self, entry: TierEntryInput) -> dict[str, JsonValue]:
        return {"model_name": entry.model_name, "litellm_params": dict(entry.litellm_params)}
