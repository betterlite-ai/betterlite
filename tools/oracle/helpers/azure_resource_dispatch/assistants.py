from typing import Final

import litellm
from pydantic import JsonValue

from azure_assistant_definitions import run as definitions_run
from azure_assistants import run as assistants_run
from azure_message_attachments import run as messages_run
from azure_thread_messages import run as threads_run

from .contracts import (
    ASSISTANTS_IN,
    DEFINITIONS_IN,
    FIELDS,
    MESSAGES_IN,
    ROWS,
    STRING,
    THREADS_IN,
    arguments,
    assistant_reply,
    first_step,
    raise_raised,
)

litellm.suppress_debug_info = True

DEFINITION_CALLS: Final[dict[str, str]] = {
    "definitions_create": "create_assistant",
    "definitions_list": "list_assistants",
    "definitions_delete": "delete_assistant",
}
MESSAGE_CALLS: Final[dict[str, str]] = {
    "message_create": "append_message",
    "message_list": "list_messages",
}


def assistant_value(kind: str, row: dict[str, JsonValue]) -> JsonValue:
    error = row.get("error")
    if error is not None:
        raise_raised(error)
    result = FIELDS.validate_python(row["result"])
    if kind in {"definitions_list", "message_list"}:
        return {"count": len(ROWS.validate_python(result["data"]))}
    if kind == "definitions_delete":
        return {"deleted": result["deleted"]}
    if kind == "run_poll":
        return {"status": result["status"]}
    return {"id": result["id"]}


def definition_answer(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    payload = arguments(step)
    payload["reply"] = assistant_reply(step)
    if kind == "definitions_create":
        payload["data"] = {"model": "gpt-4o"}
    if kind == "definitions_delete":
        payload["assistant_id"] = STRING.validate_python(step["target_id"])
    variant = DEFINITION_CALLS[kind]
    answer = definitions_run(DEFINITIONS_IN.validate_python({variant: [payload]}))
    return first_step(answer, "steps")


def thread_answer(step: dict[str, JsonValue]) -> JsonValue:
    payload = arguments(step)
    payload["reply"] = assistant_reply(step)
    payload["messages"] = [{"role": "user", "content": "hello"}]
    answer = threads_run(THREADS_IN.validate_python({"thread_create": [payload]}))
    return first_step(answer, "steps")


def message_answer(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    payload = arguments(step)
    payload["reply"] = assistant_reply(step)
    payload["thread_id"] = STRING.validate_python(step["target_id"])
    if kind == "message_create":
        payload["data"] = {"role": "user", "content": "hello"}
    variant = MESSAGE_CALLS[kind]
    answer = messages_run(MESSAGES_IN.validate_python({variant: [payload]}))
    return first_step(answer, "steps")


def run_answer(step: dict[str, JsonValue]) -> JsonValue:
    payload = arguments(step)
    payload["run"] = {
        "thread_id": "thread_1",
        "assistant_id": "asst_1",
        "model": "gpt-4o-mini",
        "instructions": "base",
    }
    payload["replies"] = ROWS.validate_python(step["replies"])
    answer = assistants_run(ASSISTANTS_IN.validate_python({"run_poll": [payload]}))
    return first_step(answer, "steps")


def one_step(kind: str, step: dict[str, JsonValue]) -> JsonValue:
    if kind in DEFINITION_CALLS:
        row = definition_answer(kind, step)
    elif kind == "thread_create":
        row = thread_answer(step)
    elif kind in MESSAGE_CALLS:
        row = message_answer(kind, step)
    else:
        row = run_answer(step)
    return assistant_value(kind, row)
