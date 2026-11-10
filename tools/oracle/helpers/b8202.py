import asyncio
import json
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.litellm_core_utils.url_utils import SSRFError
from litellm.proxy.a2a import discovery

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
BLOCKED_ADDRESS: str = "URL targets a blocked address (10.0.0.5)"


class Reply:
    def __init__(self, status: int, body: str) -> None:
        self.status_code: int = status
        self.body: str = body

    def json(self) -> Json:
        return cast(Json, json.loads(self.body))


class Script:
    def __init__(self, replies: list[Object]) -> None:
        self.replies: list[Object] = replies
        self.urls: list[Json] = []

    async def get(self, client: object, url: str, *, headers: object = None) -> Reply:
        del client, headers
        self.urls.append(url)
        index = len(self.urls) - 1
        if index >= len(self.replies):
            raise httpx.ConnectError("no reply scripted")
        reply = self.replies[index]
        error = reply.get("error")
        if error == "ssrf":
            raise SSRFError(BLOCKED_ADDRESS)
        if error == "transport":
            raise httpx.ConnectError("connection refused")
        return Reply(cast(int, reply.get("status", 200)), cast(str, reply.get("body", "")))


def fake_client(**_kwargs: object) -> object:
    return object()


def replies(input_value: Object) -> list[Object]:
    value = input_value.get("replies", [])
    if not isinstance(value, list):
        raise TypeError("replies must be a list")
    parsed: list[Object] = []
    for entry in value:
        if not isinstance(entry, dict):
            raise TypeError("a reply must be an object")
        parsed.append(entry)
    return parsed


def discovery_mode(input_value: Object) -> discovery.DiscoveryMode:
    mode = input_value.get("mode")
    if mode is None:
        return discovery.DiscoveryMode.WELL_KNOWN_FALLBACK
    return discovery.DiscoveryMode(cast(str, mode))


def params(input_value: Object) -> dict[str, object]:
    value = input_value.get("params")
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise TypeError("params must be an object or null")
    return cast(dict[str, object], value)


def run(input_value: Object) -> Json:
    operation = input_value.get("operation")
    script = Script(replies(input_value))
    with (
        patch.object(discovery, "async_safe_get", script.get),
        patch.object(discovery, "get_async_httpx_client", fake_client),
    ):
        try:
            card = asyncio.run(
                discovery.fetch_well_known_card(
                    cast(str, input_value.get("base_url", "")),
                    discovery_mode=discovery_mode(input_value),
                    params=params(input_value),
                )
            )
        except discovery.AgentCardDiscoveryError:
            if operation == "urls" and script.urls:
                return script.urls
            raise
    if operation == "urls":
        return script.urls
    return cast(Json, card)
