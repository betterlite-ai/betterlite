import asyncio
from collections.abc import AsyncIterator
from typing import TypeAlias, cast

from litellm.llms.bedrock.messages.invoke_transformations.anthropic_claude3_transformation import (
    AmazonAnthropicClaudeMessagesConfig,
    AmazonAnthropicClaudeMessagesStreamDecoder,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


async def promote(
    chunks: list[Json],
) -> list[Json]:
    async def source() -> AsyncIterator[bytes | dict[str, Json]]:
        for chunk in chunks:
            if isinstance(chunk, dict):
                yield chunk
            else:
                yield cast(bytes, chunk)

    promoted = AmazonAnthropicClaudeMessagesConfig._promote_message_stop_usage(  # noqa: SLF001 # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType, reportPrivateUsage]
        source()
    )
    return [cast(Json, chunk) async for chunk in promoted]  # pyright: ignore[reportUnknownVariableType]


def parse_metrics(chunks: list[Json]) -> list[Json]:
    decoder = AmazonAnthropicClaudeMessagesStreamDecoder.__new__(  # pyright: ignore[reportUnknownMemberType, reportPrivateUsage]
        AmazonAnthropicClaudeMessagesStreamDecoder
    )
    parsed: list[Json] = []
    for chunk in chunks:
        if isinstance(chunk, dict):
            parsed.append(cast(Json, decoder._chunk_parser(chunk)))  # noqa: SLF001 # pyright: ignore[reportUnknownMemberType, reportPrivateUsage]
        else:
            parsed.append(chunk)
    return parsed


def run(input_value: dict[str, Json]) -> Json:
    chunks = input_value.get("events")
    if not isinstance(chunks, list):
        raise ValueError("events must be a list")
    return cast(Json, asyncio.run(promote(parse_metrics(chunks))))
