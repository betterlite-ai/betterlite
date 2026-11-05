from typing import TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates import common_utils
from litellm.types.llms.openai import (
    AllMessageValues,
    ChatCompletionAssistantMessage,
    ChatCompletionUserMessage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    messages = cast(list[AllMessageValues], input_value.get("messages", []))
    if operation == "detect_first_expected_role":
        return common_utils.detect_first_expected_role(messages)
    assistant = cast(
        ChatCompletionAssistantMessage | None,
        input_value.get("assistant_continue_message"),
    )
    user = cast(ChatCompletionUserMessage | None, input_value.get("user_continue_message"))
    ensure = cast(bool, input_value.get("ensure_alternating_roles", True))
    if operation == "get_completion_messages":
        return cast(Json, common_utils.get_completion_messages(messages, assistant, user, ensure))
    raise ValueError(f"unsupported operation {operation}")
