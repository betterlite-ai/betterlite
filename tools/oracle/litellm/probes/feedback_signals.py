from collections.abc import Mapping

from litellm.router_strategy.adaptive_router.signals import (
    SessionState,
    SignalDelta,
    Turn,
    apply_turn,
)
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false


class FunctionInput(Strict):
    name: str
    arguments: str


class ToolCallInput(Strict):
    id: str
    type: str
    function: FunctionInput


class ToolResultInput(Strict):
    content: str | None = None
    is_error: bool = False


class TurnInput(Strict):
    user_content: str | None = None
    assistant_content: str | None = None
    tool_calls: list[ToolCallInput] = Field(default_factory=list)
    tool_results: list[ToolResultInput] = Field(default_factory=list)
    response_status: int | None = None


class DeltaOutput(Strict):
    misalignment: int
    stagnation: int
    disengagement: int
    satisfaction: int
    failure: int
    loop: int
    exhaustion: int


class StateOutput(Strict):
    misalignment_count: int
    stagnation_count: int
    disengagement_count: int
    satisfaction_count: int
    failure_count: int
    loop_count: int
    exhaustion_count: int
    last_user_content: str | None
    last_assistant_content: str | None
    tool_call_history: list[str]
    turn_count: int
    last_processed_turn: int
    clean_credit_awarded: bool
    terminal_status: int | None


class Output(Strict):
    deltas: list[DeltaOutput]
    state: StateOutput


def turn_value(raw: TurnInput) -> Turn:
    calls: list[Mapping[str, JsonValue]] = [
        {
            "id": call.id,
            "type": call.type,
            "function": {"name": call.function.name, "arguments": call.function.arguments},
        }
        for call in raw.tool_calls
    ]
    results: list[Mapping[str, JsonValue]] = [
        {"content": result.content, "is_error": result.is_error} for result in raw.tool_results
    ]
    return Turn(
        user_content=raw.user_content,
        assistant_content=raw.assistant_content,
        tool_calls=calls,
        tool_results=results,
        response_status=raw.response_status,
    )


def delta_output(delta: SignalDelta) -> DeltaOutput:
    return DeltaOutput(
        misalignment=delta.misalignment,
        stagnation=delta.stagnation,
        disengagement=delta.disengagement,
        satisfaction=delta.satisfaction,
        failure=delta.failure,
        loop=delta.loop,
        exhaustion=delta.exhaustion,
    )


def state_output(state: SessionState) -> StateOutput:
    return StateOutput(
        misalignment_count=state.misalignment_count,
        stagnation_count=state.stagnation_count,
        disengagement_count=state.disengagement_count,
        satisfaction_count=state.satisfaction_count,
        failure_count=state.failure_count,
        loop_count=state.loop_count,
        exhaustion_count=state.exhaustion_count,
        last_user_content=state.last_user_content,
        last_assistant_content=state.last_assistant_content,
        tool_call_history=state.tool_call_history,
        turn_count=state.turn_count,
        last_processed_turn=state.last_processed_turn,
        clean_credit_awarded=state.clean_credit_awarded,
        terminal_status=state.terminal_status,
    )


class FeedbackSignals(Operation):
    turns: list[TurnInput] = Field(default_factory=list)

    @override
    def run(self) -> Output:
        session = SessionState(
            session_id="probe",
            router_name="probe",
            model_name="probe",
            classified_type="general",
        )
        deltas = [delta_output(apply_turn(session, turn_value(turn))) for turn in self.turns]
        return Output(deltas=deltas, state=state_output(session))
