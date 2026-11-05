from typing import TypeAlias, cast

import litellm
from litellm.litellm_core_utils.prompt_templates.factory import (
    _add_missing_tool_results,  # pyright: ignore[reportPrivateUsage]
    _declared_tool_call_ids,  # pyright: ignore[reportPrivateUsage]
    _drop_unsignable_thinking_blocks,  # pyright: ignore[reportPrivateUsage]
    _is_orphaned_tool_result,  # pyright: ignore[reportPrivateUsage]
    _iter_tool_exchange_groups,  # pyright: ignore[reportPrivateUsage]
    _sanitize_empty_text_content,  # pyright: ignore[reportPrivateUsage]
    group_tool_exchanges,
    sanitize_messages_for_tool_calling,
)
from litellm.types.llms.openai import (
    AllMessageValues,
    ChatCompletionRedactedThinkingBlock,
    ChatCompletionThinkingBlock,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "sanitize_empty_text_content":
        result = cast(
            Json,
            _sanitize_empty_text_content(cast(AllMessageValues, input_value["message"])),
        )
    elif function == "add_missing_tool_results":
        messages = cast(list[AllMessageValues], input_value["messages"])
        current_index = cast(int, input_value["current_index"])
        current_message = cast(AllMessageValues, input_value["current_message"])
        result = cast(
            Json,
            _add_missing_tool_results(current_message, messages, current_index),
        )
    elif function == "is_orphaned_tool_result":
        result = cast(
            Json,
            _is_orphaned_tool_result(
                cast(AllMessageValues, input_value["current_message"]),
                cast(list[AllMessageValues], input_value["sanitized_messages"]),
            ),
        )
    elif function == "declared_tool_call_ids":
        message = cast(AllMessageValues, input_value["message"])
        result = cast(Json, sorted(_declared_tool_call_ids(message)))
    elif function in {"group_tool_exchanges", "iter_tool_exchange_groups", "repair_indices"}:
        result = run_group_function(input_value, function)
    elif function == "sanitize_messages_for_tool_calling":
        litellm.modify_params = cast(bool, input_value["modify_params"])
        messages = cast(list[AllMessageValues], input_value["messages"])
        result = cast(Json, sanitize_messages_for_tool_calling(messages))
    elif function == "drop_unsignable_thinking_blocks":
        blocks = cast(
            list[ChatCompletionThinkingBlock | ChatCompletionRedactedThinkingBlock],
            input_value["thinking_blocks"],
        )
        result = cast(Json, _drop_unsignable_thinking_blocks(blocks))
    else:
        raise ValueError(f"unknown anthropic history function: {function}")
    return result


def run_group_function(input_value: dict[str, Json], function: str) -> Json:
    messages = cast(list[AllMessageValues], input_value["messages"])
    if function == "group_tool_exchanges":
        result = cast(Json, group_tool_exchanges(messages))
    elif function == "iter_tool_exchange_groups":
        result = cast(Json, list(_iter_tool_exchange_groups(messages)))
    elif cast(bool, input_value["modify_params"]):
        result = cast(Json, group_tool_exchanges(messages))
    else:
        result = cast(Json, [[index] for index in range(len(messages))])
    return result
