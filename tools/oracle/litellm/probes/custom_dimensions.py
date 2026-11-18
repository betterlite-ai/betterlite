import asyncio
from typing import Final, Literal

from litellm.router import Router
from litellm.router_strategy.complexity_router.complexity_router import ComplexityRouter
from litellm.router_strategy.complexity_router.config import (
    ComplexityRouterConfig,
    ComplexityTier,
    custom_pattern_work,
)
from pydantic import TypeAdapter, ValidationError
from typing_extensions import TypedDict, override

from probes.protocol import Operation, ProbeError, Strict

_TIER: Final = TypeAdapter(ComplexityTier)


class DimensionInput(TypedDict, total=False):
    name: str
    weight: float
    keywords: list[str]
    patterns: list[str]
    scoring_mode: str


class ConfigInput(TypedDict, total=False):
    dimension_weights: dict[str, float] | None
    custom_dimensions: list[DimensionInput]
    classifier_type: str


class Entry(Strict):
    work: int | None = None
    message: str | None = None


class Output(Strict):
    results: list[Entry] | None = None
    work: int | None = None
    accepted: bool | None = None
    tier: str | None = None
    score: str | None = None
    signals: list[str] | None = None
    cause: str | None = None


def _entry(pattern: str) -> Entry:
    try:
        work = custom_pattern_work(pattern)
    except ValueError as error:
        raise ProbeError("invalid_request", str(error)) from error
    match work:
        case str():
            return Entry(message=work)
        case int():
            return Entry(work=work)


class CustomDimensions(Operation):
    function: Literal["pattern_work", "validate", "classify"]
    config: ConfigInput | None = None
    patterns: list[str] = []
    prompt: str = ""
    system_prompt: str | None = None

    @override
    def run(self) -> Output:
        if self.function == "pattern_work":
            return Output(results=[_entry(pattern) for pattern in self.patterns])
        raw: ConfigInput = {} if self.config is None else self.config
        if self.function == "validate":
            try:
                configured = ComplexityRouterConfig.model_validate(raw)
            except ValidationError:
                return Output(accepted=False)
            work = sum(dimension.pattern_work() for dimension in configured.custom_dimensions)
            return Output(accepted=True, work=work)
        try:
            router = ComplexityRouter(
                model_name="probe",
                litellm_router_instance=Router(),
                complexity_router_config=raw,
            )
            outcome = asyncio.run(router.aclassify(self.prompt, self.system_prompt))
        except ValidationError as error:
            raise ProbeError(
                "invalid_request", "; ".join(detail["msg"] for detail in error.errors())
            ) from error
        score = outcome.score
        return Output(
            tier=_TIER.validate_python(outcome.tier).name,
            score=None if score is None else f"{score:.6f}",
            signals=list(outcome.signals),
            cause=outcome.cause,
        )
