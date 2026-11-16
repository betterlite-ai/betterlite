from collections.abc import Mapping
from typing import Literal, Protocol, TypedDict, TypeVar

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
from typing_extensions import ReadOnly

OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
GROUPS: TypeAdapter[list[JsonValue] | None] = TypeAdapter(list[JsonValue] | None)
SETTINGS: TypeAdapter[dict[str, RetryPolicy | JsonValue]] = TypeAdapter(
    dict[str, RetryPolicy | JsonValue]
)
ArgsValue = TypeVar("ArgsValue")


class Initial(TypedDict, total=False):
    routing_strategy: RoutingStrategyName
    routing_strategy_args: dict[str, JsonValue]
    routing_groups: list[RoutingGroup | dict[str, JsonValue]]


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


# Recording goes through one instance the recorders share, so update_settings' order is kept
class Calls(list[tuple[str, JsonValue]]):
    def strategy_init(
        self,
        routing_strategy: RoutingStrategy | str | None,
        routing_strategy_args: dict[str, JsonValue] | None,
    ) -> None:
        call: dict[str, JsonValue] = {
            "routing_strategy": JSON.validate_python(routing_strategy),
            "routing_strategy_args": routing_strategy_args,
        }
        self.append(("routing_strategy_init", call))

    def pre_call_checks(self, checks: OptionalPreCallChecks | None) -> None:
        self.append(("set_optional_pre_call_checks", JSON.validate_python(checks)))

    def updated_args(self) -> None:
        self.append(("apply_updated_routing_strategy_args", None))

    def groups(
        self, groups_input: list[RoutingGroup | dict[str, JsonValue]] | None
    ) -> None:
        self.append(("init_routing_groups", GROUPS.validate_python(groups_input)))

    def lar1(
        self,
        _router: Router,
        routing_strategy_args: Mapping[str, ArgsValue] | None = None,
    ) -> None:
        self.append(("apply_lar1", JSON.validate_python(routing_strategy_args)))


class SettingsRouter(Protocol):
    def update_settings(self, **kwargs: JsonValue) -> None: ...

    def get_settings(self) -> dict[str, RetryPolicy | JsonValue]: ...


class GuardrailView(TypedDict, total=False):
    guardrail_name: ReadOnly[str]
    id: ReadOnly[str | None]


class GuardrailRouter(Protocol):
    def get_available_guardrail(self, guardrail_name: str) -> GuardrailView: ...


def build_settings_router(initial: Initial) -> SettingsRouter:
    strategy: RoutingStrategyName = "simple-shuffle"
    given_strategy = initial.get("routing_strategy")
    if given_strategy is not None:
        strategy = given_strategy
    args: dict[str, JsonValue] = {}
    given_args = initial.get("routing_strategy_args")
    if given_args is not None:
        args = given_args
    groups: list[RoutingGroup | dict[str, JsonValue]] | None = initial.get(
        "routing_groups"
    )
    return Router(
        routing_strategy=strategy,
        routing_strategy_args=args,
        routing_groups=groups,
    )


def build_guardrail_router(guardrails: list[GuardrailEntry]) -> GuardrailRouter:
    router: GuardrailRouter = Router()
    object.__setattr__(router, "guardrail_list", guardrails)
    return router


def settings_output(input_value: SettingsInput) -> dict[str, JsonValue]:
    router = build_settings_router(input_value["initial"])
    calls = Calls()
    object.__setattr__(router, "routing_strategy_init", calls.strategy_init)
    object.__setattr__(router, "set_optional_pre_call_checks", calls.pre_call_checks)
    object.__setattr__(router, "_apply_updated_routing_strategy_args", calls.updated_args)
    object.__setattr__(router, "_init_routing_groups", calls.groups)
    object.__setattr__(lar1_routing, "apply_lar1_routing_strategy", calls.lar1)
    kwargs = dict(input_value["kwargs"])
    error: dict[str, JsonValue] | None = None
    try:
        router.update_settings(**kwargs)
    except (TypeError, ValueError) as exc:
        error = {"type": type(exc).__name__, "message": str(exc)}
    shown = OBJECT.validate_json(SETTINGS.dump_json(router.get_settings()))
    listed: list[JsonValue] = [[step, argument] for step, argument in calls]
    return {
        "settings": {name: shown[name] for name in kwargs if name in shown},
        "calls": listed,
        "error": error,
    }


def run_guardrail(input_value: GuardrailInput) -> dict[str, JsonValue]:
    router = build_guardrail_router(input_value["guardrails"])
    picked = router.get_available_guardrail(input_value["guardrail_name"])
    return {"guardrail_name": picked.get("guardrail_name"), "id": picked.get("id")}


def run(input_value: SettingsInput | GuardrailInput) -> dict[str, JsonValue]:
    if input_value["function"] == "settings":
        return settings_output(input_value)
    return run_guardrail(input_value)
