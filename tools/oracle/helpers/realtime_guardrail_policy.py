from typing import Literal, TypeAlias, TypedDict

import litellm
from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.realtime_streaming import RealTimeStreaming
from litellm.types.guardrails import GuardrailEventHooks
from pydantic import JsonValue, TypeAdapter

WIRE = TypeAdapter(JsonValue)


class CallbackSpec(TypedDict):
    custom: bool
    runs_for: list[str]


class SelectionScenario(TypedDict):
    callbacks: list[CallbackSpec]
    request_data: dict[str, JsonValue]
    event_hooks: list[str]


class SelectionInput(TypedDict):
    function: Literal["transcription_guardrail_selection"]
    scenarios: list[SelectionScenario]


class ShapesInput(TypedDict):
    function: Literal["disable_auto_response_shapes"]
    backend_uses_beta_protocol: list[bool]


class SessionInput(TypedDict):
    sent: bool
    callbacks: list[CallbackSpec]
    request_data: dict[str, JsonValue]
    setup_messages: list[str]


class InjectionInput(TypedDict):
    function: Literal["setup_guardrail_injection"]
    sessions: list[SessionInput]


Input: TypeAlias = SelectionInput | ShapesInput | InjectionInput


class Probe(RealTimeStreaming):
    def __init__(
        self, request_data: dict[str, JsonValue], *, beta: bool, sent: bool
    ) -> None:
        self.request_data = request_data
        self._backend_uses_beta_protocol = beta
        self._guardrail_turn_detection_update_sent = sent

    def disable_message(self) -> str:
        return self._make_disable_auto_response_message()

    def has_audio(self) -> bool:
        return self._has_audio_transcription_guardrails()

    def has_hooks(self, hooks: list[GuardrailEventHooks]) -> bool:
        return self._has_realtime_guardrails_for_event_hooks(hooks)

    def inject(self, message: str) -> str:
        return self._maybe_inject_guardrail_auto_response_disable(message)

    def sent(self) -> bool:
        return self._guardrail_turn_detection_update_sent


def callbacks_from(specs: list[CallbackSpec]) -> list[CustomLogger]:
    callbacks: list[CustomLogger] = []
    for spec in specs:
        if not spec["custom"]:
            callbacks.append(CustomLogger())
            continue
        hooks = [GuardrailEventHooks(name) for name in spec["runs_for"]]
        callbacks.append(
            CustomGuardrail(guardrail_name=None, event_hook=hooks, default_on=True)
        )
    return callbacks


def selection(input_value: SelectionInput) -> JsonValue:
    scenarios: list[JsonValue] = []
    for scenario in input_value["scenarios"]:
        litellm.callbacks = [*callbacks_from(scenario["callbacks"])]
        probe = Probe(scenario["request_data"], beta=True, sent=False)
        hooks = [GuardrailEventHooks(name) for name in scenario["event_hooks"]]
        scenarios.append({"audio": probe.has_audio(), "hooks": probe.has_hooks(hooks)})
    return {"scenarios": scenarios}


def shapes(input_value: ShapesInput) -> JsonValue:
    messages: list[JsonValue] = [
        WIRE.validate_json(Probe({}, beta=beta, sent=False).disable_message())
        for beta in input_value["backend_uses_beta_protocol"]
    ]
    return {"messages": messages}


def injection(input_value: InjectionInput) -> JsonValue:
    sessions: list[JsonValue] = [
        session_facts(session) for session in input_value["sessions"]
    ]
    return {"sessions": sessions}


def session_facts(session: SessionInput) -> JsonValue:
    litellm.callbacks = [*callbacks_from(session["callbacks"])]
    probe = Probe(session["request_data"], beta=True, sent=session["sent"])
    messages: list[JsonValue] = [
        message_facts(probe, text) for text in session["setup_messages"]
    ]
    return {"sent": probe.sent(), "messages": messages}


def message_facts(probe: Probe, text: str) -> JsonValue:
    injected = probe.inject(text)
    try:
        message: JsonValue | None = WIRE.validate_json(injected)
    except ValueError:
        message = None
    return {
        "changed": injected is not text,
        "sent": probe.sent(),
        "message": message,
    }


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "transcription_guardrail_selection":
        return selection(input_value)
    if input_value["function"] == "disable_auto_response_shapes":
        return shapes(input_value)
    return injection(input_value)
