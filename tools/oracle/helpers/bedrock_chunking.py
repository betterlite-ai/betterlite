import json
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


def chunk_result(chunk: dict[str, Json]) -> BedrockContentChunkResult:
    return BedrockContentChunkResult(
        response=cast(BedrockGuardrailResponse, chunk["response"]),
        content=tuple(cast(list[BedrockContentItem], chunk["content"])),
        fragment_group_size=cast(int, chunk["fragment_group_size"]),
    )


def run(input_value: dict[str, Json]) -> Json:
    ((operation, arguments),) = cast(dict[str, dict[str, Json]], input_value["step"]).items()
    content = cast(list[BedrockContentItem], arguments.get("content"))
    result: object
    match operation:
        case "bin_pack":
            budget = cast(int, arguments["budget"])
            result = BedrockGuardrail._bin_pack_bedrock_content(content, budget)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        case "split":
            result = BedrockGuardrail._split_bedrock_content(content)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        case "nearest":
            result = BedrockGuardrail._nearest_whitespace_split_index(cast(str, arguments["text"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        case "merge":
            chunks = [
                chunk_result(chunk) for chunk in cast(list[dict[str, Json]], arguments["chunks"])
            ]
            result = BedrockGuardrail._merge_bedrock_guardrail_responses(chunks)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        case "grounding":
            result = BedrockGuardrail._content_uses_contextual_grounding(content)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        case "too_large":
            result = BedrockGuardrail._is_input_too_large_error(arguments["detail"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        case _:
            raise InputError(f"unsupported operation {operation}")
    return cast(Json, json.loads(json.dumps(result)))
