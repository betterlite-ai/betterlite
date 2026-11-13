from collections.abc import Generator
from contextlib import contextmanager
from types import SimpleNamespace
from typing import Literal, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.base_llm.guardrail_translation.utils import (
    anthropic_tool_name,
    anthropic_tool_names,
    effective_scan_only_tool_results_for_guardrail,
    effective_skip_system_message_for_guardrail,
    effective_skip_tool_message_for_guardrail,
    filter_messages_by_skip_flags,
    merge_guardrailed_scoped_messages,
    merge_returned_tools_into_request_tools,
    message_slot_texts,
    message_text_slot_count,
    message_with_slot_texts,
    openai_tool_name,
    scoped_structured_message_indices,
)
from litellm.types.llms.openai import AllMessageValues as Msg

from b3663.usage import Json


class Flags(TypedDict):
    skip_system: bool | None
    skip_tool: bool | None
    scan_only: bool | None
    global_skip_system: bool
    global_skip_tool: bool


class SlotsInput(TypedDict):
    function: Literal["slots"]
    message: dict[str, Json]
    texts: list[str]


class ScopeInput(TypedDict):
    function: Literal["scope"]
    messages: list[Json]
    flags: Flags


class ToolNamesInput(TypedDict):
    function: Literal["tool_names"]
    tool: Json


class MergeToolsInput(TypedDict):
    function: Literal["merge_tools"]
    extractor: Literal["openai", "anthropic"]
    request: list[Json] | None
    returned: list[Json]


class MergeMessagesInput(TypedDict):
    function: Literal["merge_messages"]
    full: list[Json]
    scoped_indices: list[int]
    returned: list[Json]


Input: TypeAlias = SlotsInput | ScopeInput | ToolNamesInput | MergeToolsInput | MergeMessagesInput


@contextmanager
def global_skip_flags(flags: Flags) -> Generator[None]:
    saved_system = litellm.skip_system_message_in_guardrail
    saved_tool = litellm.skip_tool_message_in_guardrail
    litellm.skip_system_message_in_guardrail = flags["global_skip_system"]
    litellm.skip_tool_message_in_guardrail = flags["global_skip_tool"]
    try:
        yield
    finally:
        litellm.skip_system_message_in_guardrail = saved_system
        litellm.skip_tool_message_in_guardrail = saved_tool


def scope(input_value: ScopeInput) -> Json:
    flags = input_value["flags"]
    messages = input_value["messages"]
    guardrail = SimpleNamespace(
        skip_system_message_in_guardrail=flags["skip_system"],
        skip_tool_message_in_guardrail=flags["skip_tool"],
        scan_only_tool_results=flags["scan_only"],
    )
    with global_skip_flags(flags):
        skip_system = effective_skip_system_message_for_guardrail(guardrail)
        skip_tool = effective_skip_tool_message_for_guardrail(guardrail)
        kept, changed = filter_messages_by_skip_flags(guardrail, cast(list[Msg], messages))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
    scan_only = effective_scan_only_tool_results_for_guardrail(guardrail)
    indices = scoped_structured_message_indices(
        cast(list[Msg], messages),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
        scan_only_tool_results=scan_only,
        skip_system=skip_system,
        skip_tool=skip_tool,
    )
    return {
        "skip_system": skip_system,
        "skip_tool": skip_tool,
        "scan_only": scan_only,
        "indices": [*indices],
        "filtered": cast(list[Json], [*kept]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
        "changed": changed,
    }


def slots(input_value: SlotsInput) -> Json:
    message = input_value["message"]
    texts = input_value["texts"]
    rewritten = cast(Json, message_with_slot_texts(cast(Msg, message), texts))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
    return {
        "slot_texts": [*message_slot_texts(message)],
        "slot_count": message_text_slot_count(cast(Msg, message)),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
        "rewritten": rewritten,
    }


def merge_tools(input_value: MergeToolsInput) -> Json:
    match input_value["extractor"]:
        case "openai":
            return merge_returned_tools_into_request_tools(
                input_value["request"], input_value["returned"], openai_tool_name
            )
        case "anthropic":
            return merge_returned_tools_into_request_tools(
                input_value["request"], input_value["returned"], anthropic_tool_name
            )


def merge_messages(input_value: MergeMessagesInput) -> Json:
    full = cast(list[Msg], input_value["full"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
    returned = cast(list[Msg], input_value["returned"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
    merged = merge_guardrailed_scoped_messages(full, input_value["scoped_indices"], returned)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
        list[Json], merged
    )


def run(input_value: Input) -> Json:
    match input_value["function"]:
        case "slots":
            return slots(input_value)
        case "scope":
            return scope(input_value)
        case "tool_names":
            tool = input_value["tool"]
            return {
                "openai": openai_tool_name(tool),
                "anthropic": [*anthropic_tool_names(tool)],
                "anthropic_first": anthropic_tool_name(tool),
            }
        case "merge_tools":
            return merge_tools(input_value)
        case "merge_messages":
            return merge_messages(input_value)
