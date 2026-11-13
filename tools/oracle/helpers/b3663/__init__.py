from typing import Literal, TypeAlias, TypedDict, cast

from litellm.proxy.guardrails.guardrail_hooks.content_text import (
    assistant_text_from_response,
    content_to_text,
    is_all_text_parts,
    merge_rewritten_text_parts,
)

from b3663 import failures, scope, selection, usage
from b3663.usage import Json


class ContentTextInput(TypedDict):
    function: Literal["content_text"]
    content: Json
    parts: list[Json]
    new_text: str


class AssistantTextInput(TypedDict):
    function: Literal["assistant_text"]
    response: Json


Input: TypeAlias = (
    usage.BlockedUsageInput
    | scope.Input
    | ContentTextInput
    | AssistantTextInput
    | failures.Input
    | selection.Input
)


def content_text(input_value: ContentTextInput) -> Json:
    merged = merge_rewritten_text_parts(input_value["parts"], input_value["new_text"])
    plain = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/content_text.py#L47
        list[Json], merged
    )
    return {
        "text": content_to_text(input_value["content"]),
        "all_text": is_all_text_parts(input_value["content"]),
        "merged": plain,
    }


def run(input_value: Input) -> Json:
    match input_value["function"]:
        case "blocked_usage":
            return usage.run(input_value)
        case "slots" | "scope" | "tool_names" | "merge_tools" | "merge_messages":
            return scope.run(input_value)
        case "content_text":
            return content_text(input_value)
        case "assistant_text":
            return {"text": assistant_text_from_response(input_value["response"])}
        case "is_http_exception" | "enrich_exception" | "pre_call" | "unappliable":
            return failures.run(input_value)
        case "can_modify" | "proceed":
            return selection.run(input_value)
