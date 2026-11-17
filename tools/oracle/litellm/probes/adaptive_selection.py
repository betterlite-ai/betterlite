import asyncio
import random
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from random import betavariate as stock_betavariate
from typing import ClassVar, Literal

from litellm.router_strategy.adaptive_router import bandit
from litellm.router_strategy.adaptive_router.adaptive_router import AdaptiveRouter
from litellm.types.router import (
    AdaptiveRouterConfig,
    AdaptiveRouterPreferences,
    AdaptiveRouterWeights,
    RequestType,
)
from pydantic import ConfigDict, Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

Check = Literal[
    "initial_cells",
    "apply_delta",
    "normalized_cost",
    "pick",
    "pick_model",
    "pre_routing",
    "snapshot",
]


class PreferenceInput(Strict):
    model: str
    quality_tier: int
    strengths: list[str] | None = None


class CellInput(Strict):
    model: str
    alpha: str
    beta: str


class DeltaInput(Strict):
    request_type: str
    model: str
    alpha: str
    beta: str
    delta_alpha: str
    delta_beta: str


class CostInput(Strict):
    model: str
    amount: str


class CostCheckInput(Strict):
    model: str
    amount: str
    all_costs: list[str]


class WeightsInput(Strict):
    quality: str | None = None
    cost: str | None = None


class CellOutput(Strict):
    request_type: str
    model: str
    alpha: str
    beta: str
    samples: int


class ScoreOutput(Strict):
    model: str
    quality: str
    score: str


class CostScoreOutput(Strict):
    model: str
    cost_score: str


class WeightsOutput(Strict):
    quality: str
    cost: str


class CostOutput(Strict):
    model: str
    amount: str


class SnapshotCellOutput(Strict):
    request_type: str
    model: str
    alpha: str
    beta: str
    samples: int
    quality_mean: str


class SnapshotOutput(Strict):
    router_name: str
    available_models: list[str]
    weights: WeightsOutput
    model_costs: list[CostOutput]
    cells: list[SnapshotCellOutput]


class RoutingDecisionOutput(Strict):
    router_model_name: str
    router_type: str
    routed_model: str
    cause: str
    request_type: str


class RoutingOutput(Strict):
    model: str
    messages: list[dict[str, str]] | None
    routing_decision: RoutingDecisionOutput
    metadata: JsonValue
    request_type: str
    min_quality_tier: int | None


class SnapshotCellRaw(Strict):
    request_type: str
    model: str
    alpha: float
    beta: float
    samples: int
    quality_mean: float


class SnapshotWeightsRaw(Strict):
    quality: float
    cost: float


class SnapshotRaw(Strict):
    # LiteLLM's snapshot also carries the B340/B341 feedback and queue counters.
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore", strict=True, frozen=True)

    router_name: str
    available_models: list[str]
    weights: SnapshotWeightsRaw
    model_costs: dict[str, float]
    cells: list[SnapshotCellRaw]


class Output(Strict):
    cells: list[CellOutput] | None = None
    selected: str | None = None
    scores: list[ScoreOutput] | None = None
    cost_scores: list[CostScoreOutput] | None = None
    snapshot: SnapshotOutput | None = None
    routing: RoutingOutput | None = None


def parse_float(text: str, name: str) -> float:
    try:
        return float(text)
    except ValueError as error:
        raise ProbeError("invalid_request", f"{name} is not a number: {text}") from error


class ScriptedRandom(random.Random):
    values: list[float]
    position: int

    def __init__(self, samples: list[float]) -> None:
        super().__init__(0)
        self.values = samples
        self.position = 0

    @override
    def betavariate(self, _alpha: float, _beta: float) -> float:
        if self.position >= len(self.values):
            raise ProbeError("samples_exhausted", "the request has no scripted bandit sample left")
        value = self.values[self.position]
        self.position += 1
        return value


@contextmanager
def scripted_betavariate(scripted: ScriptedRandom) -> Iterator[None]:
    random.betavariate = scripted.betavariate
    try:
        yield
    finally:
        random.betavariate = stock_betavariate


class RecordingRouter(AdaptiveRouter):
    recorded: list[tuple[RequestType, int | None]]

    def __init__(
        self,
        config: AdaptiveRouterConfig,
        preferences: dict[str, AdaptiveRouterPreferences],
        costs: dict[str, float],
    ) -> None:
        super().__init__(router_name="probe", config=config, model_to_prefs=preferences, model_to_cost=costs)
        self.recorded = []

    @override
    async def pick_model(self, request_type: RequestType, min_quality_tier: int | None = None) -> str:
        self.recorded.append((request_type, min_quality_tier))
        return await super().pick_model(request_type=request_type, min_quality_tier=min_quality_tier)


class AdaptiveSelection(Operation):
    check: Check
    available_models: list[str] = Field(default_factory=list)
    preferences: list[PreferenceInput] = Field(default_factory=list)
    request_types: list[str] | None = None
    cells: list[CellInput] = Field(default_factory=list)
    deltas: list[DeltaInput] = Field(default_factory=list)
    costs: list[CostInput] = Field(default_factory=list)
    cost_checks: list[CostCheckInput] = Field(default_factory=list)
    weights: WeightsInput | None = None
    samples: list[str] = Field(default_factory=list)
    min_quality_tier: int | None = None
    request_type: str | None = None
    messages: list[dict[str, str]] | None = None
    headers: JsonValue = None
    metadata: JsonValue = None
    metadata_present: bool = False

    @override
    def run(self) -> Output:
        checks: dict[Check, Callable[[], Output]] = {
            "initial_cells": self.initial_cells,
            "apply_delta": self.apply_deltas,
            "normalized_cost": self.normalized_costs,
            "pick": self.pick,
            "pick_model": self.pick_one,
            "pre_routing": self.pre_routing,
            "snapshot": self.snapshot,
        }
        return checks[self.check]()

    def config(self) -> AdaptiveRouterConfig:
        if self.weights is None:
            return AdaptiveRouterConfig(available_models=self.available_models)
        values: dict[str, float] = {}
        if self.weights.quality is not None:
            values["quality"] = parse_float(self.weights.quality, "weights.quality")
        if self.weights.cost is not None:
            values["cost"] = parse_float(self.weights.cost, "weights.cost")
        return AdaptiveRouterConfig(
            available_models=self.available_models,
            weights=AdaptiveRouterWeights.model_validate(values),
        )

    def preference_map(self) -> dict[str, AdaptiveRouterPreferences]:
        return {
            raw.model: AdaptiveRouterPreferences(
                quality_tier=raw.quality_tier,
                strengths=[] if raw.strengths is None else [RequestType(value) for value in raw.strengths],
            )
            for raw in self.preferences
        }

    def cost_map(self) -> dict[str, float]:
        return {raw.model: parse_float(raw.amount, "cost") for raw in self.costs}

    def types(self) -> list[RequestType]:
        if self.request_types is None:
            return list(RequestType)
        return [RequestType(value) for value in self.request_types]

    def scripted(self) -> ScriptedRandom:
        return ScriptedRandom([parse_float(text, "sample") for text in self.samples])

    def initial_cells(self) -> Output:
        preferences = self.preference_map()
        types = self.types()
        cells: list[CellOutput] = []
        for raw in self.preferences:
            for request_type in types:
                cell = bandit.initial_cell(preferences[raw.model], request_type)
                cells.append(
                    CellOutput(
                        request_type=request_type.value,
                        model=raw.model,
                        alpha=f"{cell.alpha:.6f}",
                        beta=f"{cell.beta:.6f}",
                        samples=cell.total_samples,
                    )
                )
        return Output(cells=cells)

    def apply_deltas(self) -> Output:
        cells: list[CellOutput] = []
        for raw in self.deltas:
            cell = bandit.apply_delta(
                bandit.BanditCell(alpha=parse_float(raw.alpha, "alpha"), beta=parse_float(raw.beta, "beta")),
                parse_float(raw.delta_alpha, "delta_alpha"),
                parse_float(raw.delta_beta, "delta_beta"),
            )
            cells.append(
                CellOutput(
                    request_type=raw.request_type,
                    model=raw.model,
                    alpha=f"{cell.alpha:.6f}",
                    beta=f"{cell.beta:.6f}",
                    samples=cell.total_samples,
                )
            )
        return Output(cells=cells)

    def normalized_costs(self) -> Output:
        cost_scores: list[CostScoreOutput] = []
        for raw in self.cost_checks:
            amounts = [parse_float(text, "all_costs") for text in raw.all_costs]
            cost_score = bandit.normalized_cost(parse_float(raw.amount, "amount"), amounts)
            cost_scores.append(CostScoreOutput(model=raw.model, cost_score=f"{cost_score:.6f}"))
        return Output(cost_scores=cost_scores)

    def pick(self) -> Output:
        config = self.config()
        costs = self.cost_map()
        cells = {
            raw.model: bandit.BanditCell(alpha=parse_float(raw.alpha, "alpha"), beta=parse_float(raw.beta, "beta"))
            for raw in self.cells
        }
        model_costs = {model: costs.get(model, 0.0) for model in cells}
        all_costs = list(model_costs.values())
        scripted = self.scripted()
        try:
            selected = bandit.pick_best(
                cells,
                model_costs,
                quality_weight=config.weights.quality,
                cost_weight=config.weights.cost,
                rng=scripted,
            )
        except ValueError as error:
            raise ProbeError("invalid_request", str(error)) from error
        quality_weight = config.weights.quality
        cost_weight = config.weights.cost
        scores = [
            ScoreOutput(
                model=model,
                quality=f"{sample:.6f}",
                score=f"{bandit.score(sample, model_costs[model], all_costs, quality_weight, cost_weight):.6f}",
            )
            for model, sample in zip(cells, scripted.values[: len(cells)], strict=True)
        ]
        return Output(selected=selected, scores=scores)

    def pick_one(self) -> Output:
        router = RecordingRouter(self.config(), self.preference_map(), self.cost_map())
        request_type = RequestType.GENERAL if self.request_type is None else RequestType(self.request_type)
        try:
            with scripted_betavariate(self.scripted()):
                selected = asyncio.run(
                    router.pick_model(request_type=request_type, min_quality_tier=self.min_quality_tier)
                )
        except ValueError as error:
            raise ProbeError("invalid_request", str(error)) from error
        return Output(selected=selected)

    def pre_routing(self) -> Output:
        router = RecordingRouter(self.config(), self.preference_map(), self.cost_map())
        kwargs: dict[str, JsonValue] = {"headers": self.headers}
        if self.metadata_present:
            kwargs["metadata"] = self.metadata
        try:
            with scripted_betavariate(self.scripted()):
                response = asyncio.run(
                    router.async_pre_routing_hook(model="probe", request_kwargs=kwargs, messages=self.messages)
                )
        except ValueError as error:
            raise ProbeError("invalid_request", str(error)) from error
        if response is None:
            raise ProbeError("invalid_request", "the adaptive hook returned no response")
        if response.messages != self.messages:
            raise ProbeError("invalid_request", "the adaptive hook changed the messages it was given")
        decision = response.routing_decision
        if decision is None:
            raise ProbeError("invalid_request", "the adaptive hook returned no routing decision")
        metadata = kwargs.get("metadata")
        routing_decision = RoutingDecisionOutput.model_validate(dict(decision))
        return Output(
            routing=RoutingOutput(
                model=response.model,
                messages=self.messages,
                routing_decision=routing_decision,
                metadata=metadata,
                request_type=routing_decision.request_type,
                min_quality_tier=router.recorded[0][1],
            )
        )

    def snapshot(self) -> Output:
        router = AdaptiveRouter(
            router_name="probe",
            config=self.config(),
            model_to_prefs=self.preference_map(),
            model_to_cost=self.cost_map(),
        )
        snapshot = SnapshotRaw.model_validate(asyncio.run(router.get_state_snapshot()))
        cells = [
            SnapshotCellOutput(
                request_type=cell.request_type,
                model=cell.model,
                alpha=f"{cell.alpha:.6f}",
                beta=f"{cell.beta:.6f}",
                samples=cell.samples,
                quality_mean=f"{cell.quality_mean:.6f}",
            )
            for cell in snapshot.cells
        ]
        model_costs = [
            CostOutput(model=model, amount=f"{amount:.6f}") for model, amount in snapshot.model_costs.items()
        ]
        return Output(
            snapshot=SnapshotOutput(
                router_name=snapshot.router_name,
                available_models=snapshot.available_models,
                weights=WeightsOutput(
                    quality=f"{snapshot.weights.quality:.6f}",
                    cost=f"{snapshot.weights.cost:.6f}",
                ),
                model_costs=model_costs,
                cells=cells,
            )
        )
