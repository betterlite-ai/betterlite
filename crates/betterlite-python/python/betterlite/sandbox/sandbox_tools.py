import os
from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite._endpoints import CallValue, call_native, to_call_payload
from betterlite._native import NativeDecisionError, NativeDecisionSession

# The registry lives in the native session, so the module keeps only the one session that holds it.
SESSION: Final = NativeDecisionSession("{}")
REGISTER: Final = "register_tools"
RESOLVE: Final = "resolve_tool"

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


def register_sandbox_tools(tools: list[dict[str, JsonValue]]) -> None:
    environment: JsonValue = JSON.validate_python(os.environ.copy())
    entries: JsonValue = JSON.validate_python(tools)
    arguments: dict[str, CallValue] = {"environment": environment, "tools": entries}
    call_native(
        SESSION.call,
        to_call_payload({"operation": REGISTER, "arguments": arguments}),
        NativeDecisionError,
    )


def resolve_sandbox_tool(name: str) -> dict[str, JsonValue] | None:
    answer = call_native(
        SESSION.call,
        to_call_payload({"operation": RESOLVE, "arguments": {"name": name}}),
        NativeDecisionError,
    )
    if answer is None:
        return None
    return OBJECT.validate_python(answer)


def clear_sandbox_tools() -> None:
    call_native(
        SESSION.call,
        to_call_payload({"operation": REGISTER, "arguments": {"tools": []}}),
        NativeDecisionError,
    )
