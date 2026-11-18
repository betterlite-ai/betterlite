import asyncio
from typing import Final, Literal

from litellm.router import Router
from litellm.router_strategy.complexity_router.complexity_router import ComplexityRouter
from litellm.router_strategy.complexity_router.config import (
    ComplexityRouterConfig,
    ComplexityTier,
    custom_pattern_work,
)
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

_CONFIG: Final = TypeAdapter(ComplexityRouterConfig)
_TIER: Final = TypeAdapter(ComplexityTier)


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
    config: dict[str, JsonValue] | None = None
    patterns: list[str] = []
    prompt: str = ""
    system_prompt: str | None = None

    @override
    def run(self) -> Output:
        if self.function == "pattern_work":
            return Output(results=[_entry(pattern) for pattern in self.patterns])
        # the case spells betterlite's key names for the shared complexity keys; LiteLLM reads the same names for these
        raw: dict[str, JsonValue] = {} if self.config is None else self.config
        if self.function == "validate":
            try:
                configured = _CONFIG.validate_python(raw)
            except ValidationError:
                return Output(accepted=False)
            return Output(accepted=True, work=sum(dimension.pattern_work() for dimension in configured.custom_dimensions))
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
