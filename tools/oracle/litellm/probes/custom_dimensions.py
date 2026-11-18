import asyncio
import warnings
from typing import Final, Literal

from litellm.router import Router
from litellm.router_strategy import complexity_router
from pydantic import Field, TypeAdapter, ValidationError
from typing_extensions import TypedDict, override

from probes.protocol import Operation, ProbeError, Strict

_TIER: Final = TypeAdapter(complexity_router.ComplexityTier)
_CONFIG: Final = TypeAdapter(complexity_router.ComplexityRouterConfig)
# the package exports the model and its dimension type; a validated instance lends the dimension class
_DIMENSION: Final = _CONFIG.validate_python(
    {"custom_dimensions": [{"name": "probe", "weight": 1.0, "patterns": ["a"]}]}
).custom_dimensions[0]


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


def _rejection(error: ValidationError, pattern: str) -> str:
    message = error.errors()[0]["msg"]
    return message.removeprefix(f"Value error, custom dimension pattern {pattern!r} ")


def _outcome(pattern: str) -> int | str:
    # model_construct skips the field bounds, so the pattern reaches the same cost walk validation uses
    work = _DIMENSION.model_construct(
        name="probe", weight=1.0, patterns=(pattern,), keywords=(), scoring_mode="binary"
    ).pattern_work()
    if work:
        return work
    # the cost walk already ran above; asking validation again for its refusal must not repeat its warnings
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        try:
            _ = _CONFIG.validate_python(
                {"custom_dimensions": [{"name": "probe", "weight": 1.0, "patterns": [pattern]}]}
            )
        except ValidationError as error:
            return _rejection(error, pattern)
    raise ProbeError("invalid_request", f"pattern {pattern!r} reported neither work nor a refusal")


def _entry(pattern: str) -> Entry:
    outcome = _outcome(pattern)
    match outcome:
        case str():
            return Entry(message=outcome)
        case int():
            return Entry(work=outcome)


class CustomDimensions(Operation):
    function: Literal["pattern_work", "validate", "classify"]
    config: ConfigInput | None = None
    patterns: list[str] = Field(default_factory=list)
    prompt: str = ""
    system_prompt: str | None = None

    @override
    def run(self) -> Output:
        if self.function == "pattern_work":
            return Output(results=[_entry(pattern) for pattern in self.patterns])
        raw: ConfigInput = {} if self.config is None else self.config
        if self.function == "validate":
            try:
                configured = _CONFIG.validate_python(raw)
            except ValidationError:
                return Output(accepted=False)
            work = sum(dimension.pattern_work() for dimension in configured.custom_dimensions)
            return Output(accepted=True, work=work)
        try:
            router = complexity_router.ComplexityRouter(
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
