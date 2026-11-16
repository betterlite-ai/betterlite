from collections.abc import Mapping
from typing import Literal, Protocol, TypedDict, TypeVar, runtime_checkable

from litellm.router import Router
from litellm.router_strategy import lar1_routing
from litellm.types.litellm_params import RoutingStrategyName
from litellm.types.router import (
    GuardrailLiteLLMParams,
    OptionalPreCallChecks,
    RetryPolicy,
    RoutingGroup,
    RoutingStrategy,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
GROUPS: TypeAdapter[list[JsonValue] | None] = TypeAdapter(list[JsonValue] | None)
SETTINGS: TypeAdapter[dict[str, RetryPolicy | JsonValue]] = TypeAdapter(
    dict[str, RetryPolicy | JsonValue]
)
ArgsValue = TypeVar("ArgsValue")

# Every recorder appends here in call order, since update_settings runs them in one pass
CALLS: list[tuple[str, JsonValue]] = []


class Initial(TypedDict, total=False):
    routing_strategy: RoutingStrategyName
    routing_strategy_args: dict[str, JsonValue]
    routing_groups: list[dict[str, JsonValue]]


class SettingsInput(TypedDict):
    function: Literal["settings"]
    initial: Initial
    kwargs: list[tuple[str, JsonValue]]


class GuardrailEntry(TypedDict):
    guardrail_name: str
    id: str | None
    litellm_params: GuardrailLiteLLMParams


class GuardrailInput(TypedDict):
    function: Literal["guardrail"]
    guardrails: list[GuardrailEntry]
    guardrail_name: str


class _RecordingRouter(Router):
    recording: bool = False

    def start_recording(self) -> None:
        self.recording = True

    def record_lar1(
        self, router: Router, routing_strategy_args: Mapping[str, ArgsValue] | None = None
    ) -> None:
        router.routing_strategy = "lar1"
        CALLS.append(("apply_lar1", JSON.validate_python(routing_strategy_args)))

    @override
    def routing_strategy_init(
        self, routing_strategy: RoutingStrategy | str, routing_strategy_args: dict[str, JsonValue]
    ) -> None:
        if not self.recording:
            super().routing_strategy_init(routing_strategy, routing_strategy_args)
            return
        call: dict[str, JsonValue] = {
            "routing_strategy": JSON.validate_python(routing_strategy),
            "routing_strategy_args": routing_strategy_args,
        }
        CALLS.append(("routing_strategy_init", call))

    @override
    def set_optional_pre_call_checks(
        self, optional_pre_call_checks: OptionalPreCallChecks | None
    ) -> None:
        if not self.recording:
            super().set_optional_pre_call_checks(optional_pre_call_checks)
            return
        CALLS.append(("set_optional_pre_call_checks", JSON.validate_python(optional_pre_call_checks)))

    @override
    def _apply_updated_routing_strategy_args(self) -> None:
        if not self.recording:
            super()._apply_updated_routing_strategy_args()
            return
        CALLS.append(("apply_updated_routing_strategy_args", None))

    @override
    def _init_routing_groups(
        self, groups_input: list[RoutingGroup | dict[str, JsonValue]] | None
    ) -> None:
        if not self.recording:
            super()._init_routing_groups(groups_input)
            return
        CALLS.append(("init_routing_groups", GROUPS.validate_python(groups_input)))


@runtime_checkable
class SettingsRouter(Protocol):
    def __init__(
        self,
        *,
        model_list: list[JsonValue],
        routing_strategy: RoutingStrategyName = "simple-shuffle",
        routing_strategy_args: dict[str, JsonValue] | None = None,
        routing_groups: list[dict[str, JsonValue]] | None = None,
    ) -> None: ...

    def start_recording(self) -> None: ...

    def record_lar1(
        self, router: Router, routing_strategy_args: Mapping[str, ArgsValue] | None = None
    ) -> None: ...

    def update_settings(self, **kwargs: JsonValue) -> None: ...

    def get_settings(self) -> dict[str, RetryPolicy | JsonValue]: ...


@runtime_checkable
class GuardrailRouter(Protocol):
    def __init__(
        self, *, model_list: list[JsonValue], guardrail_list: list[GuardrailEntry]
    ) -> None: ...

    def get_available_guardrail(self, guardrail_name: str) -> GuardrailEntry: ...


SETTINGS_ROUTER: type[SettingsRouter] = TypeAdapter(type[SettingsRouter]).validate_python(
    _RecordingRouter
)
GUARDRAIL_ROUTER: type[GuardrailRouter] = TypeAdapter(type[GuardrailRouter]).validate_python(
    Router
)


def settings_output(input_value: SettingsInput) -> dict[str, JsonValue]:
    CALLS.clear()
    router = SETTINGS_ROUTER(model_list=[], **input_value["initial"])
    router.start_recording()
    lar1_routing.apply_lar1_routing_strategy = router.record_lar1
    kwargs = dict(input_value["kwargs"])
    error: dict[str, JsonValue] | None = None
    try:
        router.update_settings(**kwargs)
    except (TypeError, ValueError) as exc:
        error = {"type": type(exc).__name__, "message": str(exc)}
    shown = OBJECT.validate_json(SETTINGS.dump_json(router.get_settings()))
    return {
        "settings": {name: shown[name] for name in kwargs if name in shown},
        "calls": [[step, argument] for step, argument in CALLS],
        "error": error,
    }


def run(input_value: SettingsInput | GuardrailInput) -> dict[str, JsonValue]:
    if input_value["function"] == "settings":
        return settings_output(input_value)
    router = GUARDRAIL_ROUTER(model_list=[], guardrail_list=input_value["guardrails"])
    picked = router.get_available_guardrail(input_value["guardrail_name"])
    return {"guardrail_name": picked["guardrail_name"], "id": picked["id"]}
