import asyncio
import time as time_module
from collections.abc import Callable, Coroutine, Mapping, Sequence
from typing import Final, TypedDict
from unittest.mock import patch

from litellm.proxy.common_utils import cache_aware_routing as cache_common
from litellm.proxy.common_utils.cache_aware_routing import CacheAwareChoice
from litellm.router import Router
from litellm.router_strategy.complexity_router import complexity_router
from litellm.router_strategy.complexity_router.complexity_router import ComplexityRouter
from litellm.router_strategy.complexity_router.config import ComplexityRouterConfig
from litellm.types.router import PreRoutingHookResponse
from litellm.types.utils import StandardLoggingRoutingDecision
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import Unpack, override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

REAL_TIME: Final = (time_module.time, time_module.monotonic)
_MAP: Final = TypeAdapter(dict[str, JsonValue])
_APPLY: Callable[..., Coroutine[object, object, PreRoutingHookResponse | None]] = TypeAdapter(
    Callable[..., Coroutine[object, object, PreRoutingHookResponse | None]]
).validate_python(ComplexityRouter.__dict__["_apply_prompt_cache_routing"])
_FIT: Callable[..., object] = TypeAdapter(Callable[..., object]).validate_python(
    complexity_router.__dict__["_RequestContextFit"]
)


class ParamsInput(Strict):
    tier: str
    model: str
    params: dict[str, JsonValue] = Field(default_factory=dict)


class ResponseInput(Strict):
    model: str
    routing_decision: dict[str, JsonValue] | None = None
    session_affinity_ttl_seconds: int | None = None
    litellm_params: dict[str, JsonValue] | None = None


class MessageInput(Strict):
    role: str
    content: str | None = None
    name: str | None = None


class FitFactsInput(Strict):
    window: int | None = None
    has_unknown: bool = False


class FitInput(Strict):
    facts: dict[str, FitFactsInput] = Field(default_factory=dict)
    needed: int | None = None
    buffer: str


class ChoiceInput(Strict):
    model: str
    tier: str
    deployment_id: str
    original_cost: str
    estimated_cost: str


class CallOutput(Strict):
    model: str
    decision_present: bool
    output_tokens: int
    messages_present: bool
    kwargs_keys: list[str]


class ResponseOutput(Strict):
    model: str
    messages_present: bool
    routing_decision: dict[str, JsonValue] | None
    session_affinity_ttl_seconds: int | None
    litellm_params: dict[str, JsonValue] | None


class Output(Strict):
    calls: int
    call: CallOutput | None
    response: ResponseOutput | None


class ChooseCall(TypedDict):
    router: Router
    config: ComplexityRouterConfig
    params_for_model: Callable[[str, str], Mapping[str, object]]
    response: PreRoutingHookResponse | None
    request_kwargs: Mapping[str, object]
    messages: Sequence[Mapping[str, object]] | None


# the prediction itself: record what choose_cached_model asked for, wait as the case says, then answer
class SlowChoice:
    choice: CacheAwareChoice | None
    delay_seconds: float
    observations: list[CallOutput]

    def __init__(self, choice: CacheAwareChoice | None, delay_ms: int) -> None:
        self.choice = choice
        self.delay_seconds = delay_ms / 1000
        self.observations = []

    async def __call__(self, **kwargs: Unpack[ChooseCall]) -> CacheAwareChoice | None:
        response = kwargs["response"]
        self.observations.append(
            CallOutput(
                model="" if response is None else response.model,
                decision_present=response is not None and response.routing_decision is not None,
                output_tokens=kwargs["config"].cache_aware_routing_output_tokens,
                messages_present=kwargs["messages"] is not None,
                kwargs_keys=list(kwargs["request_kwargs"].keys()),
            )
        )
        await asyncio.sleep(self.delay_seconds)
        return self.choice


class CacheCostRouting(Operation):
    cache_aware_routing: bool = False
    cache_aware_routing_output_tokens: int = 1024
    cache_aware_routing_timeout_ms: int = 2000
    tier_labels: dict[str, str] = Field(default_factory=dict)
    tiers: dict[str, str] = Field(default_factory=dict)
    params: list[ParamsInput] = Field(default_factory=list)
    response: ResponseInput | None = None
    messages: list[MessageInput] | None = None
    request_kwargs: dict[str, JsonValue] = Field(default_factory=dict)
    context_fit: FitInput
    choice: ChoiceInput | None = None
    chooser_delay_ms: int = 0

    @override
    def run(self) -> Output:
        # the probe protocol freezes time.monotonic, and the prediction deadline is a timer on it
        frozen = (time_module.time, time_module.monotonic)
        time_module.time, time_module.monotonic = REAL_TIME
        try:
            return self._drive()
        finally:
            time_module.time, time_module.monotonic = frozen

    def _drive(self) -> Output:
        router = Router()
        complexity = ComplexityRouter(
            model_name="probe",
            litellm_router_instance=router,
            complexity_router_config=self._configured(),
        )
        chooser = SlowChoice(self._choice(), self.chooser_delay_ms)
        with patch.object(cache_common, "_choose_cached_model", chooser):
            applied = asyncio.run(
                _APPLY(
                    complexity,
                    self._response(),
                    self._message_values(),
                    self.request_kwargs,
                    self._fit(),
                )
            )
        return Output(
            calls=len(chooser.observations),
            call=chooser.observations[-1] if chooser.observations else None,
            response=None if applied is None else self._response_output(applied),
        )

    def _configured(self) -> dict[str, object]:
        return {
            "tiers": {tier: self._tier_value(tier, model) for tier, model in self.tiers.items()},
            "tier_labels": dict(self.tier_labels),
            "max_tokens_from_tier_model": False,
            "cache_aware_routing": self.cache_aware_routing,
            "cache_aware_routing_output_tokens": self.cache_aware_routing_output_tokens,
            "cache_aware_routing_timeout_ms": self.cache_aware_routing_timeout_ms,
        }

    def _tier_value(self, tier: str, model: str) -> dict[str, object]:
        entry = next(
            (declared for declared in self.params if declared.tier == tier and declared.model == model),
            None,
        )
        if entry is None:
            return {"model_name": model}
        return {"model_name": model, "litellm_params": dict(entry.params)}

    def _response(self) -> PreRoutingHookResponse | None:
        declared = self.response
        if declared is None:
            return None
        value: dict[str, object] = {"model": declared.model, "messages": self._message_values()}
        if declared.routing_decision is not None:
            value["routing_decision"] = declared.routing_decision
        if declared.session_affinity_ttl_seconds is not None:
            value["session_affinity_ttl_seconds"] = declared.session_affinity_ttl_seconds
        if declared.litellm_params is not None:
            value["litellm_params"] = declared.litellm_params
        return PreRoutingHookResponse.model_validate(value)

    def _message_values(self) -> list[dict[str, object]] | None:
        if self.messages is None:
            return None
        values: list[dict[str, object]] = []
        for message in self.messages:
            value: dict[str, object] = {"role": message.role}
            if message.content is not None:
                value["content"] = message.content
            if message.name is not None:
                value["name"] = message.name
            values.append(value)
        return values

    def _fit(self) -> object:
        facts: dict[str, tuple[int | None, bool]] = {
            model: (declared.window, declared.has_unknown)
            for model, declared in self.context_fit.facts.items()
        }
        return _FIT(facts, self.context_fit.needed, float(self.context_fit.buffer))

    def _choice(self) -> CacheAwareChoice | None:
        declared = self.choice
        if declared is None:
            return None
        return CacheAwareChoice(
            model=declared.model,
            tier=declared.tier,
            deployment_id=declared.deployment_id,
            original_cost=float(declared.original_cost),
            estimated_cost=float(declared.estimated_cost),
        )

    def _response_output(self, applied: PreRoutingHookResponse) -> ResponseOutput:
        decision = applied.routing_decision
        params = applied.litellm_params
        return ResponseOutput(
            model=applied.model,
            messages_present=applied.messages is not None,
            routing_decision=None if decision is None else self._decision_value(decision),
            session_affinity_ttl_seconds=applied.session_affinity_ttl_seconds,
            litellm_params=None if params is None else _MAP.validate_python(dict(params)),
        )

    # the apply stamps a tuple for signals and the frozen tier params as a mappingproxy
    def _decision_value(self, decision: StandardLoggingRoutingDecision) -> dict[str, JsonValue]:
        value = dict(decision)
        signals = decision.get("signals")
        if signals is not None:
            value["signals"] = list(signals)
        tier_params = decision.get("tier_litellm_params")
        if tier_params is not None:
            value["tier_litellm_params"] = dict(tier_params)
        return _MAP.validate_python(value)
