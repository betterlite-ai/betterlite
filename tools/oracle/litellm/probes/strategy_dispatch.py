from __future__ import annotations

import datetime as dt
from typing import TYPE_CHECKING, Literal, cast
from unittest.mock import patch

import litellm
from litellm._logging import verbose_router_logger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.router import Router
from litellm.router_strategy.simple_shuffle import simple_shuffle
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict
from probes.single_candidate import Deployment  # noqa: TC001 - pydantic builds the model from it at import

if TYPE_CHECKING:
    from litellm.types.litellm_params import RoutingStrategyName
    from litellm.types.router import DeploymentTypedDict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

SELECTOR_STRATEGIES: dict[str, str] = {
    "LowestCostLoggingHandler": "cost-based-routing",
    "LowestLatencyLoggingHandler": "latency-based-routing",
    "LeastBusyLoggingHandler": "least-busy",
    "LowestTPMLoggingHandler": "usage-based-routing",
    "LowestTPMLoggingHandler_v2": "usage-based-routing-v2",
}

class State(Strict):
    selector: str | None
    callbacks: list[str]
    input_callbacks: list[str]


class ValidateOutput(Strict):
    function: Literal["validate"] = "validate"
    valid: bool


class ReinitOutput(Strict):
    function: Literal["reinit"] = "reinit"
    before: State
    after: State
    old_registered_after: bool
    selector_replaced: bool
    override_cached_before: bool
    override_rebuilt_after: bool


class OverrideOutput(Strict):
    function: Literal["override"] = "override"
    override_strategy: str | None
    ignored: bool
    supported: list[str]
    selector: str | None
    selector_is_default: bool
    selector_cached: bool
    bound: bool


class DispatchOutput(Strict):
    function: Literal["dispatch"] = "dispatch"
    strategy: str
    selected: str


def _required(value: str | None, name: str) -> str:
    if value is None:
        raise ProbeError("invalid_request", f"the strategy_dispatch scenario needs {name}")
    return value


def _selector_strategy(selector: object) -> str | None:
    return SELECTOR_STRATEGIES.get(type(selector).__name__)


def _strategies(selectors: list[object]) -> list[str]:
    names: list[str] = []
    for selector in selectors:
        name = _selector_strategy(selector)
        if name is not None:
            names.append(name)
    return names


def _default_selector(router: Router, strategy: str) -> object | None:
    if strategy == "least-busy":
        return router.leastbusy_logger
    if strategy == "usage-based-routing":
        return router.lowesttpm_logger
    if strategy == "usage-based-routing-v2":
        return router.lowesttpm_logger_v2
    if strategy == "latency-based-routing":
        return router.lowestlatency_logger
    if strategy == "cost-based-routing":
        return router.lowestcost_logger
    return None


def _reset() -> None:
    litellm.callbacks = []
    litellm.input_callback = []


def _state(router: Router, strategy: str) -> State:
    callbacks = cast("list[object]", litellm.callbacks)
    input_callbacks = cast("list[object]", litellm.input_callback)
    default = _default_selector(router, strategy)
    selector = None if default is None else _selector_strategy(default)
    return State(
        selector=selector,
        callbacks=_strategies(callbacks),
        input_callbacks=_strategies(input_callbacks),
    )


def _model_list(deployments: list[Deployment], model: str) -> list[DeploymentTypedDict]:
    return [
        {
            "model_name": model,
            "litellm_params": {
                "model": "openai/gpt-4o",
                "weight": deployment.weight,
                "rpm": deployment.rpm,
                "tpm": deployment.tpm,
            },
            "model_info": {"id": deployment.id},
        }
        for deployment in deployments
    ]


def _validate(operation: StrategyDispatch) -> ValidateOutput:
    value = _required(operation.value, "value")
    try:
        Router._validate_routing_strategy(value)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    except ValueError:
        return ValidateOutput(valid=False)
    return ValidateOutput(valid=True)


def _reinit(operation: StrategyDispatch) -> ReinitOutput:
    strategy = _required(operation.strategy, "strategy")
    next_strategy = _required(operation.next_strategy, "next_strategy")
    _reset()
    router = Router(
        model_list=_model_list(operation.deployments, "gpt"),
        routing_strategy=cast("RoutingStrategyName", strategy),
    )
    before = _state(router, strategy)
    cached = router._get_override_strategy_selector("cost-based-routing")  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    again = router._get_override_strategy_selector("cost-based-routing")  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    old = _default_selector(router, strategy)
    router.routing_strategy_init(  # pyright: ignore[reportUnknownMemberType]
        routing_strategy=next_strategy, routing_strategy_args={}
    )
    after = _state(router, next_strategy)
    new = _default_selector(router, next_strategy)
    callbacks = cast("list[object]", litellm.callbacks)
    old_registered_after = old is not None and any(callback is old for callback in callbacks)
    rebuilt = router._get_override_strategy_selector("cost-based-routing")  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return ReinitOutput(
        before=before,
        after=after,
        old_registered_after=old_registered_after,
        selector_replaced=new is not old,
        override_cached_before=cached is again,
        override_rebuilt_after=rebuilt is not cached,
    )


def _override(operation: StrategyDispatch) -> OverrideOutput:
    strategy = _required(operation.strategy, "strategy")
    _reset()
    router = Router(
        model_list=_model_list(operation.deployments, "gpt"),
        routing_strategy=cast("RoutingStrategyName", strategy),
    )
    request_kwargs: dict[str, object] = {}
    raw = operation.routing_strategy
    if raw is not None:
        request_kwargs["routing_strategy"] = raw
    captured: list[list[str]] = []

    def capture(_message: str, _raw: object, supported: list[str]) -> None:
        captured.append(supported)

    with patch.object(verbose_router_logger, "warning", side_effect=capture):
        override = router._get_request_routing_strategy_override(request_kwargs)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    selector = None
    selector_is_default = False
    selector_cached = True
    bound = False
    if override is not None:
        chosen = router._get_override_strategy_selector(override)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        second = router._get_override_strategy_selector(override)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        selector_is_default = chosen is not None and chosen is _default_selector(router, strategy)
        selector_cached = chosen is second
        selector = None if chosen is None else _selector_strategy(chosen)
        logging_object = Logging(
            model="gpt",
            messages=[{"role": "user", "content": "hi"}],
            stream=False,
            call_type="completion",
            start_time=dt.datetime.now(dt.UTC),
            litellm_call_id="strategy-dispatch-probe",
            function_id="strategy-dispatch-probe",
        )
        router._bind_override_selector_to_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            override, chosen, {"litellm_logging_obj": logging_object}
        )
        dynamic = cast("list[object]", logging_object.dynamic_input_callbacks or [])
        bound = any(callback is chosen for callback in dynamic)
    return OverrideOutput(
        override_strategy=override,
        ignored=raw is not None and override is None,
        supported=captured[0] if captured else [],
        selector=selector,
        selector_is_default=selector_is_default,
        selector_cached=selector_cached,
        bound=bound,
    )


def _dispatch(operation: StrategyDispatch) -> DispatchOutput:
    strategy = _required(operation.strategy, "strategy")
    model = _required(operation.model, "model")
    _reset()
    entries = _model_list(operation.deployments, model)
    router = Router(model_list=entries, routing_strategy=cast("RoutingStrategyName", strategy))
    resolved, _selector = router._get_routing_context(model, {})  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    if resolved != "simple-shuffle":
        raise ProbeError(
            "invalid_request",
            "the strategy_dispatch probe selects simple-shuffle deployments only",
        )
    selected = simple_shuffle(lambda _model: None, entries, model, {})
    index = next(index for index, deployment in enumerate(entries) if deployment is selected)
    return DispatchOutput(strategy=resolved, selected=operation.deployments[index].id)


class StrategyDispatch(Operation):
    function: Literal["validate", "reinit", "override", "dispatch"]
    strategy: str | None = None
    value: str | None = None
    next_strategy: str | None = None
    routing_strategy: str | None = None
    model: str | None = None
    deployments: list[Deployment] = Field(default_factory=list)

    @override
    def run(self) -> Strict:
        if self.function == "validate":
            return _validate(self)
        if self.function == "reinit":
            return _reinit(self)
        if self.function == "override":
            return _override(self)
        return _dispatch(self)
