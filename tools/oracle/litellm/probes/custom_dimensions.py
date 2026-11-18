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
# the package exports the model and its dimension type; a validated instance gives the class
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


def _entry(pattern: str) -> Entry:
    # model_construct skips the field bounds, so the pattern reaches validation's cost walk
    work = _DIMENSION.model_construct(
        name="probe", weight=1.0, patterns=(pattern,), keywords=(), scoring_mode="binary"
    ).pattern_work()
    if work:
        return Entry(work=work)
    # the cost walk ran above, so asking validation again must not repeat the warnings
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        try:
            _ = _CONFIG.validate_python(
                {"custom_dimensions": [{"name": "probe", "weight": 1.0, "patterns": [pattern]}]}
            )
        except ValidationError as error:
            prefix = f"Value error, custom dimension pattern {pattern!r} "
            return Entry(message=error.errors()[0]["msg"].removeprefix(prefix))
    raise ProbeError("invalid_request", f"pattern {pattern!r} reported neither work nor a refusal")


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
