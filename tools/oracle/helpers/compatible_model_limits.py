import asyncio
from typing import TypeAlias, cast
from unittest.mock import AsyncMock, MagicMock

import httpx
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.llms.openai_like.model_info import get_openai_compatible_model_info

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


async def limits(call: dict[str, Json]) -> Json:
    request = httpx.Request("GET", "https://api.test/v1/models")
    client = MagicMock()
    client.get = AsyncMock(
        return_value=httpx.Response(200, content=cast(str, call["body"]).encode(), request=request)
    )
    found = await get_openai_compatible_model_info(
        model=cast(str, call["model"]),
        api_base="https://api.test",
        headers={},
        client=client,
        cache=InMemoryCache(),
    )
    return cast(Json, dict(found))


def run(value: dict[str, Json]) -> Json:
    return [asyncio.run(limits(call)) for call in cast(list[dict[str, Json]], value["calls"])]
