from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates.mid_conversation_system import (
    anthropic_system_messages,
    place_mid_conversation_system,
    split_leading_system_run,
    system_message_as_user,
)
from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _split_leading_system_run(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    leading, rest = split_leading_system_run(messages)
    return cast(Json, [list(leading), list(rest)])


def _anthropic_system_messages(input_value: dict[str, Json]) -> Json:
    message = cast(AllMessageValues, input_value["message"])
    return cast(Json, list(anthropic_system_messages(message)))


def _system_message_as_user(input_value: dict[str, Json]) -> Json:
    message = cast(AllMessageValues, input_value["message"])
    return cast(Json, system_message_as_user(message))


def _place_mid_conversation_system(input_value: dict[str, Json]) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    supports_mid_conversation_system = cast(bool, input_value["supports_mid_conversation_system"])
    result = place_mid_conversation_system(
        messages, supports_mid_conversation_system=supports_mid_conversation_system
    )
    return cast(Json, list(result))


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "split_leading_system_run": _split_leading_system_run,
    "anthropic_system_messages": _anthropic_system_messages,
    "system_message_as_user": _system_message_as_user,
    "place_mid_conversation_system": _place_mid_conversation_system,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    if operation not in HANDLERS:
        raise ValueError(f"unsupported operation {operation}")
    return HANDLERS[operation](input_value)
