from collections.abc import Callable, Sequence
from typing import Final
from unittest.mock import patch

from litellm.router import Router
from litellm.router_strategy.adaptive_router import bandit
from litellm.router_strategy.adaptive_router.adaptive_router import AdaptiveRouter
from litellm.router_strategy.adaptive_router.bandit import (
    BanditCell,
    apply_delta,
    initial_cell,
)
from litellm.router_strategy.complexity_router import complexity_router
from litellm.router_strategy.complexity_router.complexity_router import ComplexityRouter
from litellm.router_strategy.complexity_router.config import ComplexityTier
from litellm.types.router import AdaptiveRouterPreferences, RequestType
from pydantic import Field, JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

from probes.protocol import InjectedRandom, Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true
# pyright: reportMissingTypeStubs=false, reportUnknownMemberType=false, reportUnusedCallResult=false

_DEFAULT_PREFS: Final = AdaptiveRouterPreferences(quality_tier=2, strengths=[])
_METADATA: Final = TypeAdapter(dict[str, JsonValue])

Pick = Callable[
    [
        ComplexityRouter,
        ComplexityTier | str,
        str,
        dict[str, JsonValue] | None,
        ComplexityTier | str | None,
        ComplexityTier | str | None,
        frozenset[str] | None,
    ],
    str,
]

_SOFT_FLOOR_PICK: Final[Pick] = TypeAdapter(Pick).validate_python(
    ComplexityRouter.__dict__["_soft_floor_pick"]
)

_ENSURE_ADAPTIVE: Final[Callable[[ComplexityRouter], AdaptiveRouter | None]] = TypeAdapter(
    Callable[[ComplexityRouter], AdaptiveRouter | None]
).validate_python(ComplexityRouter.__dict__["_ensure_adaptive_router"])


class WeightsInput(Strict):
    quality: str | None = None
    cost: str | None = None


class PoolInput(Strict):
    tier: str
    models: list[str]


class DeploymentInput(Strict):
    model_name: str
    quality_tier: int | None = None
    strengths: list[str] = Field(default_factory=list)
    params_cost: str | None = None
    info_cost: str | None = None


class ObservationInput(Strict):
    request_type: str
    model: str
    delta_alpha: str
    delta_beta: str


class RawCandidate(Strict):
    model: str
    total_samples: int | None = None
    quality_sample: float | None = None
    cost_score: float | None = None
    tier_distance: int | None = None
    score: float | None = None


class RawDecision(Strict):
    phase: str
    classified_tier: str
    request_type: str
    eligible_mode: str
    quality_weight: float
    cost_weight: float
    tier_distance_penalty: float
    chosen_model: str
    candidates: list[RawCandidate]


_RAW_DECISION: Final = TypeAdapter(RawDecision)


class CandidateOutput(Strict):
    model: str
    total_samples: int | None = None
    quality_sample: str | None = None
    cost_score: str | None = None
    tier_distance: int | None = None
    score: str | None = None


class DecisionOutput(Strict):
    phase: str
    classified_tier: str
    request_type: str
    eligible_mode: str
    quality_weight: str
    cost_weight: str
    tier_distance_penalty: str
    chosen_model: str
    candidates: list[CandidateOutput]


class Output(Strict):
    chosen_model: str
    decision: DecisionOutput | None = None
    decision_keys: list[str] = Field(default_factory=list)


def parse_number(text: str, name: str) -> float:
    try:
        return float(text)
    except ValueError as error:
        raise ProbeError("invalid_request", f"{name} is not a number: {text}") from error


def complexity_tier(name: str) -> ComplexityTier | str:
    try:
        return ComplexityTier(name)
    except ValueError:
        return name


# the shipped random.choice draws from getrandbits, so the case's own stream decides the pick
class InjectedChoice:
    def __init__(self, draws: InjectedRandom) -> None:
        self.draws: InjectedRandom = draws

    def choice(self, seq: Sequence[str]) -> str:
        if not seq:
            raise ProbeError("invalid_request", "the pick drew from an empty sequence")
        return seq[int(self.draws.random() * len(seq))]


# thompson_sample reads .betavariate off this module's random, so the case scripts the draws
class ScriptedBeta:
    def __init__(self, samples: list[float]) -> None:
        self.pending: list[float] = list(reversed(samples))

    def betavariate(self, alpha: float, beta: float) -> float:
        _ = (alpha, beta)
        if not self.pending:
            raise ProbeError(
                "samples_exhausted", "the pick asked for more bandit samples than the case scripts"
            )
        return self.pending.pop()


# the case's learned cells: apply_delta over the cold prior the router already computed
def seeded_cells(
    adaptive: AdaptiveRouter,
    observations: list[ObservationInput],
) -> dict[tuple[RequestType, str], BanditCell]:
    cells = {
        (request_type, model): initial_cell(
            adaptive.model_to_prefs.get(model, _DEFAULT_PREFS), request_type
        )
        for request_type in RequestType
        for model in adaptive.config.available_models
    }
    for observation in observations:
        key = (RequestType(observation.request_type), observation.model)
        updated = apply_delta(
            cells[key],
            parse_number(observation.delta_alpha, "delta_alpha"),
            parse_number(observation.delta_beta, "delta_beta"),
        )
        cells[key] = updated
    return cells


def candidate_output(raw: RawCandidate) -> CandidateOutput:
    return CandidateOutput(
        model=raw.model,
        total_samples=raw.total_samples,
        quality_sample=None if raw.quality_sample is None else f"{raw.quality_sample:.6f}",
        cost_score=None if raw.cost_score is None else f"{raw.cost_score:.6f}",
        tier_distance=raw.tier_distance,
        score=None if raw.score is None else f"{raw.score:.6f}",
    )


def decision_output(raw: RawDecision) -> DecisionOutput:
    return DecisionOutput(
        phase=raw.phase,
        classified_tier=raw.classified_tier,
        request_type=raw.request_type,
        eligible_mode=raw.eligible_mode,
        quality_weight=f"{raw.quality_weight:.6f}",
        cost_weight=f"{raw.cost_weight:.6f}",
        tier_distance_penalty=f"{raw.tier_distance_penalty:.6f}",
        chosen_model=raw.chosen_model,
        candidates=[candidate_output(candidate) for candidate in raw.candidates],
    )


class ComplexityAdaptivePlacement(Operation):
    adaptive: bool = False
    weights: WeightsInput | None = None
    tier_distance_penalty: str | None = None
    adaptive_eligible: str | None = None
    pools: list[PoolInput] = Field(default_factory=list)
    deployments: list[DeploymentInput] = Field(default_factory=list)
    default_model: str | None = None
    classified_tier: str
    user_message: str
    hard_floor: str | None = None
    hard_ceiling: str | None = None
    fit_filter: list[str] | None = None
    observations: list[ObservationInput] = Field(default_factory=list)
    samples: list[str] = Field(default_factory=list)
    kwargs_present: bool | None = None

    @override
    def run_with(self, generator: InjectedRandom) -> Output:
        router = self._router()
        complexity = ComplexityRouter(
            model_name="probe",
            litellm_router_instance=router,
            complexity_router_config=self._configured(),
        )
        adaptive = _ENSURE_ADAPTIVE(complexity)
        if adaptive is not None:
            object.__setattr__(adaptive, "_cells", seeded_cells(adaptive, self.observations))
        kwargs: dict[str, JsonValue] | None = {} if self.kwargs_present is not False else None
        scripted = ScriptedBeta([parse_number(text, "sample") for text in self.samples])
        with (
            patch.object(complexity_router, "random", InjectedChoice(generator)),
            patch.object(bandit, "random", scripted),
        ):
            model = _SOFT_FLOOR_PICK(
                complexity,
                complexity_tier(self.classified_tier),
                self.user_message,
                kwargs,
                None if self.hard_floor is None else complexity_tier(self.hard_floor),
                None if self.hard_ceiling is None else complexity_tier(self.hard_ceiling),
                None if self.fit_filter is None else frozenset(self.fit_filter),
            )
        return self._output(model, kwargs)

    def _router(self) -> Router:
        router = Router()
        model_list: list[dict[str, JsonValue]] = []
        indices: dict[str, list[int]] = {}
        for deployment in self.deployments:
            indices.setdefault(deployment.model_name, []).append(len(model_list))
            model_list.append(self._deployment(deployment))
        object.__setattr__(router, "model_list", model_list)
        object.__setattr__(router, "model_name_to_deployment_indices", indices)
        return router

    def _deployment(self, deployment: DeploymentInput) -> dict[str, JsonValue]:
        info: dict[str, JsonValue] = {}
        if deployment.quality_tier is not None:
            strengths: list[JsonValue] = list(deployment.strengths)
            preferences: dict[str, JsonValue] = {
                "quality_tier": deployment.quality_tier,
                "strengths": strengths,
            }
            info["adaptive_router_preferences"] = preferences
        if deployment.info_cost is not None:
            info["input_cost_per_token"] = parse_number(deployment.info_cost, "info_cost")
        params: dict[str, JsonValue] = {"model": deployment.model_name}
        if deployment.params_cost is not None:
            params["input_cost_per_token"] = parse_number(deployment.params_cost, "params_cost")
        return {"litellm_params": params, "model_info": info}

    def _configured(self) -> dict[str, object]:
        config: dict[str, object] = {
            "tiers": {pool.tier: list(pool.models) for pool in self.pools},
            "default_model": self.default_model,
            "adaptive": self.adaptive,
            "adaptive_eligible": "all" if self.adaptive_eligible is None else self.adaptive_eligible,
            "tier_distance_penalty": (
                0.5
                if self.tier_distance_penalty is None
                else parse_number(self.tier_distance_penalty, "tier_distance_penalty")
            ),
        }
        weights: dict[str, float] = {}
        if self.weights is not None:
            if self.weights.quality is not None:
                weights["quality"] = parse_number(self.weights.quality, "weights.quality")
            if self.weights.cost is not None:
                weights["cost"] = parse_number(self.weights.cost, "weights.cost")
        if weights:
            config["adaptive_weights"] = weights
        return config

    def _output(self, model: str, kwargs: dict[str, JsonValue] | None) -> Output:
        raw_object: dict[str, JsonValue] | None = None
        if kwargs is not None:
            try:
                metadata = _METADATA.validate_python(kwargs.get("metadata"))
                raw_object = _METADATA.validate_python(metadata.get("adaptive_router_decision"))
            except ValidationError:
                raw_object = None
        if raw_object is None:
            return Output(chosen_model=model)
        return Output(
            chosen_model=model,
            decision=decision_output(_RAW_DECISION.validate_python(raw_object)),
            decision_keys=sorted(raw_object),
        )
