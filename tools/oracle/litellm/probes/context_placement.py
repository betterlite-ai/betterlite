import asyncio
from collections.abc import Callable, Coroutine, Mapping, Sequence
from typing import Final, TypeAlias

from litellm.router import Router
from litellm.router_strategy.complexity_router import complexity_router
from litellm.router_strategy.complexity_router.complexity_router import ComplexityRouter
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

FitResult: TypeAlias = tuple[Mapping[str, tuple[int | None, bool]], int | None, float]
PlacementResult: TypeAlias = tuple[object, tuple[str, ...], frozenset[str]] | None
AppliedResult: TypeAlias = tuple[object, tuple[str, ...], object | None]

_FIT: Callable[..., Coroutine[object, object, FitResult]] = TypeAdapter(
    Callable[..., Coroutine[object, object, FitResult]]
).validate_python(ComplexityRouter.__dict__["_request_context_fit"])

_PLACEMENT: Callable[..., Coroutine[object, object, PlacementResult]] = TypeAdapter(
    Callable[..., Coroutine[object, object, PlacementResult]]
).validate_python(ComplexityRouter.__dict__["_context_window_placement"])

_APPLY: Callable[..., AppliedResult] = TypeAdapter(Callable[..., AppliedResult]).validate_python(
    complexity_router.__dict__["_apply_context_placement"]
)

_OUT_OF_BAND: Callable[..., str] = TypeAdapter(Callable[..., str]).validate_python(
    ComplexityRouter.__dict__["_out_of_band_request_text"]
)

_BYTE_BOUND: Callable[..., int] = TypeAdapter(Callable[..., int]).validate_python(
    ComplexityRouter.__dict__["_request_byte_upper_bound"]
)

_OUTPUT_CEILING: Callable[..., int | None] = TypeAdapter(Callable[..., int | None]).validate_python(
    ComplexityRouter.__dict__["_group_output_ceiling"]
)

_TIER_NAME: Callable[..., str] = TypeAdapter(Callable[..., str]).validate_python(
    complexity_router.__dict__["_tier_name"]
)

_MAP: Final = TypeAdapter(dict[str, JsonValue])


class ConfigInput(Strict):
    enabled: bool = False
    buffer: str | None = None


class DeploymentInput(Strict):
    model: str | None = None
    custom_llm_provider: str | None = None
    max_input_tokens: int | None = None
    max_output_tokens: int | None = None
    router_max_input_tokens: int | None = None
    router_max_output_tokens: int | None = None


class GroupInput(Strict):
    model: str
    deployments: list[DeploymentInput] = Field(default_factory=list)


class WindowFact(Strict):
    window: int | None
    has_unknown: bool


class PlacementOutput(Strict):
    tier: str
    allowed_models: list[str]
    holdable_models: list[str]


class AppliedOutput(Strict):
    tier: str
    signals: list[str]
    original_tier: str | None


class Output(Strict):
    out_of_band: str
    byte_bound: int
    facts: dict[str, WindowFact]
    needed: int | None
    ceilings: dict[str, int | None]
    placement: PlacementOutput | None
    applied: AppliedOutput


# the world's get_model_list and get_router_model_info: deployments carry the map answer in probe_router
class ProbeRouter:
    def __init__(self, groups: list[GroupInput]) -> None:
        self.groups: dict[str, GroupInput] = {group.model: group for group in groups}

    def get_model_list(self, model_name: str) -> list[dict[str, JsonValue]] | None:
        group = self.groups.get(model_name)
        if group is None:
            return None
        return [self._deployment(deployment) for deployment in group.deployments]

    def get_router_model_info(
        self, deployment: dict[str, JsonValue], received_model_name: str
    ) -> dict[str, JsonValue]:
        _ = received_model_name
        mapped = _MAP.validate_python(deployment.get("probe_router"))
        declared = _MAP.validate_python(deployment.get("model_info"))
        return {**mapped, **{key: value for key, value in declared.items() if value is not None}}

    def _deployment(self, deployment: DeploymentInput) -> dict[str, JsonValue]:
        params: dict[str, JsonValue] = {"model": deployment.model or ""}
        if deployment.custom_llm_provider is not None:
            params["custom_llm_provider"] = deployment.custom_llm_provider
        model_info: dict[str, JsonValue] = {}
        if deployment.max_input_tokens is not None:
            model_info["max_input_tokens"] = deployment.max_input_tokens
        if deployment.max_output_tokens is not None:
            model_info["max_output_tokens"] = deployment.max_output_tokens
        router: dict[str, JsonValue] = {}
        if deployment.router_max_input_tokens is not None:
            router["max_input_tokens"] = deployment.router_max_input_tokens
        if deployment.router_max_output_tokens is not None:
            router["max_output_tokens"] = deployment.router_max_output_tokens
        return {"litellm_params": params, "model_info": model_info, "probe_router": router}


# a count of None is the real counter's failure contract: the gate then has no token count
class CountedTokens:
    def __init__(self, count: int | None) -> None:
        self.count: int | None = count

    async def __call__(
        self, resolved_messages: Sequence[Mapping[str, object]], request_kwargs: Mapping[str, object]
    ) -> int | None:
        _ = resolved_messages
        _ = request_kwargs
        return self.count


class ContextPlacement(Operation):
    config: ConfigInput | None = None
    tier: str
    signals: list[str] = Field(default_factory=list)
    default_model: str | None = None
    pool_override: list[str] | None = None
    pools: dict[str, list[str]]
    groups: list[GroupInput] = Field(default_factory=list)
    messages: list[str] = Field(default_factory=list)
    instructions: JsonValue | None = None
    system: JsonValue | None = None
    count: int | None = None

    @override
    def run(self) -> Output:
        router = Router()
        probe = ProbeRouter(self.groups)
        object.__setattr__(router, "get_model_list", probe.get_model_list)
        object.__setattr__(router, "get_router_model_info", probe.get_router_model_info)
        complexity = ComplexityRouter(
            model_name="probe",
            litellm_router_instance=router,
            complexity_router_config=self._configured(),
        )
        object.__setattr__(complexity, "_counted_request_tokens", CountedTokens(self.count))
        messages: list[dict[str, str]] = [
            {"role": "user", "content": text} for text in self.messages
        ]
        kwargs = self._request_kwargs()
        fit = asyncio.run(_FIT(complexity, messages, kwargs))
        facts, needed, _buffer = fit
        try:
            placement = asyncio.run(
                _PLACEMENT(
                    complexity,
                    self.tier,
                    messages,
                    kwargs,
                    None if self.pool_override is None else tuple(self.pool_override),
                    fit,
                )
            )
        except ValueError as error:
            raise ProbeError("invalid_request", str(error)) from error
        applied_tier, signals, original_tier = _APPLY(self.tier, tuple(self.signals), placement)
        return Output(
            out_of_band=_OUT_OF_BAND(kwargs),
            byte_bound=_BYTE_BOUND(complexity, messages, kwargs),
            facts={
                name: WindowFact(window=window, has_unknown=unknown)
                for name, (window, unknown) in facts.items()
            },
            needed=needed,
            ceilings={group.model: _OUTPUT_CEILING(complexity, group.model) for group in self.groups},
            placement=self._placement(placement),
            applied=AppliedOutput(
                tier=_TIER_NAME(applied_tier),
                signals=list(signals),
                original_tier=None if original_tier is None else _TIER_NAME(original_tier),
            ),
        )

    def _configured(self) -> dict[str, object]:
        configured: dict[str, object] = {
            "tiers": self.pools,
            "enable_context_window_escalation": False if self.config is None else self.config.enabled,
        }
        if self.config is not None and self.config.buffer is not None:
            configured["context_window_escalation_buffer"] = float(self.config.buffer)
        return configured

    def _request_kwargs(self) -> dict[str, object]:
        kwargs: dict[str, object] = {}
        if self.instructions is not None:
            kwargs["instructions"] = self.instructions
        body: dict[str, JsonValue] = {}
        if self.system is not None:
            body["system"] = self.system
        if body:
            kwargs["proxy_server_request"] = {"body": body}
        return kwargs

    def _placement(self, placement: PlacementResult) -> PlacementOutput | None:
        if placement is None:
            return None
        tier, allowed, holdable = placement
        return PlacementOutput(
            tier=_TIER_NAME(tier), allowed_models=list(allowed), holdable_models=sorted(holdable)
        )
