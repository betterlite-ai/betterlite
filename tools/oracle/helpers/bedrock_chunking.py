import json
from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.proxy.guardrails.guardrail_hooks.bedrock_guardrails import (
    BedrockContentChunkResult,
    BedrockGuardrail,
)
from litellm.types.proxy.guardrails.guardrail_hooks.bedrock_guardrails import (
    BedrockContentItem,
    BedrockGuardrailResponse,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class InputError(ValueError):
    pass


def run(input_value: dict[str, Json]) -> Json:
    step = input_value["step"]
    if not isinstance(step, dict):
        raise InputError("step must be an object")
    operation = step["operation"]
    names = {
        "bin_pack": "_bin_pack_bedrock_content",
        "split": "_split_bedrock_content",
        "nearest": "_nearest_whitespace_split_index",
        "merge": "_merge_bedrock_guardrail_responses",
        "grounding": "_content_uses_contextual_grounding",
        "too_large": "_is_input_too_large_error",
    }
    if not isinstance(operation, str) or operation not in names:
        raise InputError("unsupported operation")
    method = cast(Callable[..., object], getattr(BedrockGuardrail, names[operation]))
    if operation == "merge":
        chunks = cast(list[dict[str, Json]], step["chunks"])
        args: list[object] = [
            [
                BedrockContentChunkResult(
                    response=cast(BedrockGuardrailResponse, chunk["response"]),
                    content=tuple(cast(list[BedrockContentItem], chunk["content"])),
                    fragment_group_size=cast(int, chunk["fragment_group_size"]),
                )
                for chunk in chunks
            ]
        ]
    elif operation == "bin_pack":
        args = [step["content"], step.get("budget", 25_000)]
    else:
        key = (
            "text"
            if operation == "nearest"
            else "detail"
            if operation == "too_large"
            else "content"
        )
        args = [step[key]]
    return cast(Json, json.loads(json.dumps(method(*args))))
