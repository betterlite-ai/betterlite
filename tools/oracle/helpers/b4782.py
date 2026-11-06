import asyncio
import json
import os
from typing import TypeAlias, cast
from unittest import mock

import httpx
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy.agent_endpoints import databricks_oauth

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

# the Rust side swaps the same placeholder for the address of its local server
PLACEHOLDER = "UPSTREAM"
UPSTREAM_URL = "http://upstream.test"


def parse(params: dict[str, object]) -> Json:
    config = databricks_oauth.parse_databricks_oauth_config(params)
    if config is None:
        return None
    return {
        "client_id": config.client_id,
        "client_secret": config.client_secret,
        "token_url": config.token_url,
        "scope": config.scope,
        "cache_key": config.cache_key,
    }


async def drive(fields: dict[str, Json]) -> Json:
    upstream = cast(dict[str, Json], fields["upstream"])
    rounds = cast(list[dict[str, Json]], fields["rounds"])
    seen: list[Json] = []

    async def handle(request: httpx.Request) -> httpx.Response:
        seen.append(
            {
                "method": request.method,
                "path": request.url.path,
                "authorization": request.headers.get("authorization"),
                "content_type": request.headers.get("content-type"),
                "body": request.content.decode(),
            }
        )
        await asyncio.sleep(cast(int, upstream.get("delay_ms", 0)) / 1000)
        return httpx.Response(
            cast(int, upstream["status"]),
            content=cast(str, upstream["body"]),
            headers={"content-type": "application/json"},
        )

    handler = AsyncHTTPHandler()
    handler.client = httpx.AsyncClient(transport=httpx.MockTransport(handle))
    cache = databricks_oauth.databricks_app_oauth_token_cache
    cache.flush_cache()
    now = [0.0]
    cache._clock = lambda: now[0]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    headers: list[Json] = []
    with mock.patch.object(databricks_oauth, "get_async_httpx_client", return_value=handler):
        for step in rounds:
            now[0] += cast(int, step.get("advance", 0))
            params = cast(
                dict[str, object],
                json.loads(json.dumps(step["params"]).replace(PLACEHOLDER, UPSTREAM_URL)),
            )
            calls = [
                databricks_oauth.resolve_databricks_app_auth_header(params)
                for _ in range(cast(int, step.get("callers", 1)))
            ]
            results = await asyncio.gather(*calls)
            headers.append([None if found is None else found["Authorization"] for found in results])
    locks = len(cache._locks)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"headers": headers, "requests": seen, "locks": locks}


def run(input_value: dict[str, Json]) -> Json:
    env = cast(dict[str, str], input_value.get("env", {}))
    with mock.patch.dict(os.environ, env):
        if input_value["operation"] == "parse":
            return parse(cast(dict[str, object], input_value["params"]))
        return asyncio.run(drive(input_value))
