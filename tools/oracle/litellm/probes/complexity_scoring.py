import asyncio
from typing import Final, Literal

from litellm.router import Router
from litellm.router_strategy.complexity_router.complexity_router import ComplexityRouter
from litellm.router_strategy.complexity_router.config import ComplexityTier
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false, reportUnusedCallResult=false

_CONFIG: Final = TypeAdapter(dict[str, JsonValue])
_TIER: Final = TypeAdapter(ComplexityTier)


class ConfigInput(Strict):
    tiers: dict[str, str | list[str]] | None = Field(default=None, validation_alias="tier_models")
    token_thresholds: dict[str, int] | None = Field(default=None, validation_alias="complexity_thresholds")
    dimension_weights: dict[str, float] | None = None
    tier_boundaries: dict[str, float] | None = None
    reasoning_override_min_score: float | None = None
    code_keywords: list[str] | None = None
    reasoning_keywords: list[str] | None = None
    technical_keywords: list[str] | None = None
    custom_technical_keywords: list[str] | None = None
    simple_keywords: list[str] | None = None
    default_model: str | None = None


class Output(Strict):
    tier: str | None = None
    score: str | None = None
    signals: list[str] | None = None
    cause: str | None = None
    model: str | None = None


class ComplexityScoring(Operation):
    function: Literal["classify", "tier_model"]
    config: ConfigInput | None = None
    prompt: str = ""
    system_prompt: str | None = None
    tier: str = ""

    @override
    def run(self) -> Output:
        # the wire spells betterlite's tier_models and complexity_thresholds keys, and the aliases name them to the config
        configured = None if self.config is None else _CONFIG.validate_json(self.config.model_dump_json(exclude_none=True))
        router = ComplexityRouter(
            model_name="probe",
            litellm_router_instance=Router(),
            complexity_router_config=configured,
        )
        if self.function == "tier_model":
            return Output(model=router.get_model_for_tier(self.tier))
        outcome = asyncio.run(router.aclassify(self.prompt, self.system_prompt))
        score = outcome.score
        return Output(
            tier=_TIER.validate_python(outcome.tier).name,
            score=None if score is None else f"{score:.6f}",
            signals=list(outcome.signals),
            cause=outcome.cause,
        )
