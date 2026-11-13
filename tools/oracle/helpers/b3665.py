from functools import partial
from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.proxy import proxy_server
from litellm.proxy.guardrails import auto_router_compression
from litellm.types.guardrails import GuardrailEventHooks, Mode
from pydantic import JsonValue, TypeAdapter

EventHook: TypeAlias = GuardrailEventHooks | list[GuardrailEventHooks] | Mode | None


class Gate(Protocol):
    def should_run_guardrail(
        self, data: dict[str, JsonValue], event_type: GuardrailEventHooks
    ) -> bool: ...

    def get_guardrail_dynamic_request_body_params(  # ast-grep-ignore: no-long-names
        self, request_data: dict[str, JsonValue]
    ) -> dict[str, JsonValue]: ...


class GateInput(TypedDict):
    function: Literal["should_run_guardrail"]
    guardrail_name: str | None
    event_hook: JsonValue
    default_on: bool
    data: dict[str, JsonValue]
    event_type: str
    suppressed: list[str]


class HookInput(TypedDict):
    function: Literal["event_hook_is_event_type"]
    guardrail_name: str | None
    event_hook: JsonValue
    default_on: bool
    data: dict[str, JsonValue]
    event_type: str


class DynamicInput(TypedDict):
    function: Literal["dynamic_request_body_params"]
    guardrail_name: str | None
    event_hook: JsonValue
    default_on: bool
    data: dict[str, JsonValue]


Input: TypeAlias = GateInput | HookInput | DynamicInput


def build_guardrail(
    guardrail_name: str | None, wire: JsonValue, *, default_on: bool
) -> Gate:
    event_hook = TypeAdapter(EventHook).validate_python(wire)
    return CustomGuardrail(
        guardrail_name=guardrail_name, event_hook=event_hook, default_on=default_on
    )


def should_run(
    gate: Gate, data: dict[str, JsonValue], event_type: str, suppressed: list[str]
) -> bool | Literal["rejected"]:
    original = auto_router_compression.suppressed_compression_guardrails
    auto_router_compression.suppressed_compression_guardrails = partial(frozenset, suppressed)
    try:
        return gate.should_run_guardrail(data, GuardrailEventHooks(event_type))
    except ImportError:
        return "rejected"
    finally:
        auto_router_compression.suppressed_compression_guardrails = original


def run(input_value: Input) -> dict[str, JsonValue]:
    probing = input_value["function"] == "event_hook_is_event_type"
    gate = build_guardrail(
        input_value["guardrail_name"],
        input_value["event_hook"],
        default_on=input_value["default_on"] or probing,
    )
    if input_value["function"] == "event_hook_is_event_type":
        # Nothing public reaches _event_hook_is_event_type, but a default_on guardrail asks it first
        verdict = should_run(gate, {}, input_value["event_type"], [])
        return {"result": verdict is not False}
    if input_value["function"] == "should_run_guardrail":
        return {
            "result": should_run(
                gate, input_value["data"], input_value["event_type"], input_value["suppressed"]
            )
        }
    premium = proxy_server.premium_user
    proxy_server.premium_user = True
    try:
        return {"result": gate.get_guardrail_dynamic_request_body_params(input_value["data"])}
    finally:
        proxy_server.premium_user = premium
