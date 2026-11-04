from typing import TypeAlias, cast

from litellm.llms.anthropic.chat.transformation import AnthropicConfig
from pydantic import BaseModel

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def to_json(value: object) -> Json:
    if isinstance(value, BaseModel):
        return cast(Json, value.model_dump())
    if isinstance(value, list):
        return [to_json(item) for item in cast(list[object], value)]
    if isinstance(value, dict):
        fields = cast(dict[object, object], value)
        return {str(key): to_json(item) for key, item in fields.items()}
    return cast(Json, value)


def run(fields: dict[str, Json]) -> Json:
    response = fields.get("response", {"content": fields.get("content", [])})
    config = AnthropicConfig()
    result = config.extract_response_content(  # pyright: ignore[reportUnknownMemberType]
        cast(dict[str, object], response)
    )
    text, citations, thinking, reasoning, calls, web, results, compact = result
    message, calls, extra = config._resolve_json_mode_non_streaming(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast(bool | None, fields.get("json_mode")), calls
    )
    projected = config._build_provider_specific_fields(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        cast(dict[str, object], response),
        citations,
        thinking,
        web,
        results,
        compact,
        calls,
    )
    provider_fields = to_json(projected)
    return cast(Json, {
        "content": text,
        "citations": citations,
        "thinking_blocks": thinking,
        "reasoning_content": reasoning,
        "tool_calls": calls,
        "web_search_results": web,
        "tool_results": results,
        "compaction_blocks": compact,
        "extra_content": extra if message is None else message.content,
        "provider_specific_fields": provider_fields,
    })
