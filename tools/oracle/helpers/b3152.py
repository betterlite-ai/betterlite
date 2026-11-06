import asyncio
import json
from collections.abc import Awaitable, Callable
from typing import TypeAlias, cast

import httpx
from litellm.proxy.roi_calculator import github as gh
from litellm.types.roi_calculator import ROISettings
from pydantic import SecretStr

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DEFAULT_API_URL = "https://api.github.com"
PAIR_LENGTH = 2
PAGINATION_ERROR = "GitHub returned an unexpected pagination response."


class Script(httpx.AsyncBaseTransport):
    def __init__(self, replies: list[Json]) -> None:
        self.replies: list[Json] = list(replies)
        self.log: list[Json] = []

    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        await request.aread()
        self.log.append(
            {
                "method": request.method,
                "url": str(request.url.copy_with(query=None)),
                "query": [list(pair) for pair in request.url.params.multi_items()],
                "accept": request.headers.get("accept"),
                "authorization": request.headers.get("authorization"),
            }
        )
        if not self.replies:
            raise httpx.ConnectError("the script has no reply left", request=request)
        reply = self.replies.pop(0)
        if not isinstance(reply, dict):
            raise ValueError("a reply must be an object")
        if reply.get("fail"):
            raise httpx.ConnectError("the scripted transport failed", request=request)
        status = reply.get("status")
        if not isinstance(status, int) or isinstance(status, bool):
            raise ValueError("a reply needs an integer status")
        headers = header_pairs(reply.get("headers"))
        if "raw" in reply:
            content = str(reply["raw"]).encode()
        elif "body" in reply:
            content = json.dumps(reply["body"], separators=(",", ":")).encode()
        else:
            content = b""
        return httpx.Response(status, headers=headers, content=content, request=request)


def header_pairs(raw: Json) -> list[tuple[str, str]]:
    pairs: list[tuple[str, str]] = []
    if raw is None:
        return pairs
    if not isinstance(raw, list):
        raise ValueError("headers must be a list of pairs")
    for pair in raw:
        if not isinstance(pair, list) or len(pair) != PAIR_LENGTH:
            raise ValueError("a header is a name and a value")
        name, value = pair
        if not isinstance(name, str) or not isinstance(value, str):
            raise ValueError("a header name and value must be strings")
        pairs.append((name, value))
    return pairs


def text_of(value: Json, default: str) -> str:
    return value if isinstance(value, str) else default


def integer_of(value: Json, default: int) -> int:
    if isinstance(value, int) and not isinstance(value, bool):
        return value
    return default


def settings(input_value: dict[str, Json]) -> ROISettings:
    return ROISettings(
        github_api_url=text_of(input_value.get("api_url"), DEFAULT_API_URL),
        github_token=SecretStr(text_of(input_value.get("token"), "")),
    )


def init(client: gh.GitHub) -> dict[str, Json]:
    headers = client._headers  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {
        "api_url": client._api_url,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "timeout_seconds": int(client.client.timeout.read or 0),
        "accept": headers.get("Accept"),
        "authorization": headers.get("Authorization"),
    }


def params_of(input_value: dict[str, Json]) -> dict[str, str]:
    raw = input_value.get("params") or []
    if not isinstance(raw, list):
        raise ValueError("params must be a list of pairs")
    pairs: dict[str, str] = {}
    for pair in raw:
        if not isinstance(pair, list) or len(pair) != PAIR_LENGTH:
            raise ValueError("a param is a name and a value")
        name, value = pair
        if not isinstance(name, str) or not isinstance(value, str):
            raise ValueError("a param name and value must be strings")
        pairs[name] = value
    return pairs


async def request(client: gh.GitHub, input_value: dict[str, Json]) -> dict[str, Json]:
    method = input_value.get("method")
    path = input_value.get("path")
    if not isinstance(method, str) or not isinstance(path, str):
        raise ValueError("request needs a method and a path")
    response = await gh._request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        client.client,
        method,
        client._url(path),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        params=params_of(input_value),
        headers=client._headers,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    return {"status": response.status_code}


async def fetch_page(client: gh.GitHub, input_value: dict[str, Json]) -> dict[str, Json]:
    path = input_value.get("path")
    page = input_value.get("page")
    message = input_value.get("error_message")
    if not isinstance(path, str) or not isinstance(page, int) or isinstance(page, bool):
        raise ValueError("fetch_page needs a path and an integer page")
    items, has_next = await gh._fetch_page(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        client.client,
        client._url(path),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        gh._REPOSITORIES,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        params_of(input_value),
        page,
        headers=client._headers,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        error_message=text_of(message, PAGINATION_ERROR),
    )
    return {"items": [item.model_dump() for item in items], "has_next": has_next}


async def pages(client: gh.GitHub, input_value: dict[str, Json]) -> dict[str, Json]:
    path = input_value.get("path")
    limit = input_value.get("limit")
    if not isinstance(path, str) or not isinstance(limit, int) or isinstance(limit, bool):
        raise ValueError("pages needs a path and an integer limit")
    collected = await gh._collect(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        gh._pages(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            client.client,
            client._url(path),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            gh._REPOSITORIES,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            params_of(input_value),
            limit,
            headers=client._headers,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        )
    )
    return {"pages": [[item.model_dump() for item in page] for page in collected]}


async def repositories(client: gh.GitHub, input_value: dict[str, Json]) -> dict[str, Json]:
    found, has_more = await client.repositories(
        query=text_of(input_value.get("query"), ""),
        page=integer_of(input_value.get("page"), 1),
    )
    return {
        "repositories": [
            {"full_name": name, "visibility": visibility, "archived": archived}
            for name, visibility, archived in found
        ],
        "has_more": has_more,
    }


async def test_repositories(
    client: gh.GitHub, input_value: dict[str, Json]
) -> dict[str, Json]:
    repos = input_value.get("repos") or []
    if not isinstance(repos, list):
        raise ValueError("repos must be a list")
    names: list[str] = []
    for repo in repos:
        if not isinstance(repo, str):
            raise ValueError("a repository must be a string")
        names.append(repo)
    await client.test_repositories(tuple(names))
    return {}


async def profile_email(client: gh.GitHub, input_value: dict[str, Json]) -> dict[str, Json]:
    calls = input_value.get("calls") or []
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    emails: list[Json] = []
    for call in calls:
        if not isinstance(call, dict):
            raise ValueError("a profile call must be an object")
        login = call.get("login")
        fallback = call.get("fallback")
        if not isinstance(login, str):
            raise ValueError("a profile call needs a login")
        emails.append(
            await client.profile_email(login, fallback=text_of(fallback, ""))
        )
    return {"emails": emails}


Handler: TypeAlias = Callable[[gh.GitHub, dict[str, Json]], Awaitable[dict[str, Json]]]

HANDLERS: dict[str, Handler] = {
    "request": request,
    "fetch_page": fetch_page,
    "pages": pages,
    "repositories": repositories,
    "test_repositories": test_repositories,
    "profile_email": profile_email,
}


async def dispatch(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    if not isinstance(operation, str):
        raise ValueError("operation must be a string")
    script = Script(cast(list[Json], input_value.get("replies") or []))
    client = gh.GitHub(settings(input_value), transport=script)
    if operation == "init":
        payload = init(client)
    else:
        handler = HANDLERS.get(operation)
        if handler is None:
            raise ValueError(f"unknown b3152 operation: {operation}")
        payload = await handler(client, input_value)
    payload["requests"] = script.log
    return payload


def run(input_value: dict[str, Json]) -> Json:
    return asyncio.run(dispatch(input_value))
