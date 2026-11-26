import os
from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite._endpoints import CallPayload, CallValue, to_call_payload
from betterlite._native import NativeDecisionError, NativeDecisionSession
from betterlite.decisions.main import raise_decision_error

# The registry lives in the native session, so the module keeps only the one session that holds it.
SESSION: Final = NativeDecisionSession("{}")
REGISTER: Final = "register_tools"
RESOLVE: Final = "resolve_tool"

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


def call_registry(session: NativeDecisionSession, payload: CallPayload) -> JsonValue:
    try:
        answer = session.call(payload.text, payload.blobs)
    except NativeDecisionError as error:
        raise_decision_error(error)
    return JSON.validate_json(answer)


def register_sandbox_tools(tools: list[dict[str, JsonValue]]) -> None:
    environment: JsonValue = JSON.validate_python(os.environ.copy())
    entries: JsonValue = JSON.validate_python(tools)
    arguments: dict[str, CallValue] = {"environment": environment, "tools": entries}
    call_registry(
        SESSION,
        to_call_payload({"operation": REGISTER, "arguments": arguments}),
    )


def resolve_sandbox_tool(name: str) -> dict[str, JsonValue] | None:
    answer = call_registry(
        SESSION,
        to_call_payload({"operation": RESOLVE, "arguments": {"name": name}}),
    )
    if answer is None:
        return None
    return OBJECT.validate_python(answer)


def clear_sandbox_tools() -> None:
    call_registry(
        SESSION,
        to_call_payload({"operation": REGISTER, "arguments": {"tools": []}}),
    )
