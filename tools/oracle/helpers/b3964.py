import asyncio
import os
import uuid
from collections.abc import Awaitable, Callable
from itertools import count
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.integrations.galileo import GalileoObserve
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
redact_headers = cast(
    Callable[[dict[str, str] | None], dict[str, str]],
    GalileoObserve._redact_headers,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
)


class ScriptedHandler:
    def __init__(self, replies: list[Fields]) -> None:
        self.replies = replies
        self.calls: list[Json] = []

    def answer(self, method: str, url: str, call: Fields) -> httpx.Response:
        self.calls.append({"method": method, "url": url, **call})
        if not self.replies:
            message = f"request to {url} failed: no scripted reply"
            raise RuntimeError(message)
        reply = self.replies.pop(0)
        return httpx.Response(
            status_code=cast(int, reply["status"]),
            text=cast(str, reply["body"]),
            request=httpx.Request(method, url),
        )

    async def get(self, url: str, headers: Fields) -> httpx.Response:
        return self.answer("GET", url, {"headers": headers})

    async def post(
        self,
        url: str,
        headers: Fields,
        data: Fields | None = None,
        json: Fields | None = None,
    ) -> httpx.Response:
        call: Fields = {"headers": headers, "form": data}
        if json is not None:
            traces = cast(list[Fields], json["traces"])
            ids: list[Json] = [
                [trace["id"], cast(list[Fields], trace["spans"])[0]["id"]] for trace in traces
            ]
            rest: Fields = {name: value for name, value in json.items() if name != "traces"}
            call["payload"] = {"trace_ids": ids, "rest": rest}
        return self.answer("POST", url, call)


def run(input_value: Fields) -> Json:
    environment = cast(dict[str, str], input_value["environment"])
    replies = cast(list[Fields], input_value.get("replies", []))
    actions = cast(list[Fields], input_value["actions"])
    ids = count(1)
    with patch.dict(os.environ):
        for name in [name for name in os.environ if name.startswith("GALILEO_")]:
            del os.environ[name]
        os.environ.update(environment)
        logger = GalileoObserve()
    handler = ScriptedHandler(replies)
    logger.async_httpx_handler = cast(AsyncHTTPHandler, handler)
    with patch.object(uuid, "uuid4", lambda: uuid.UUID(int=next(ids))):
        results = asyncio.run(_perform(logger, actions))
    stored = logger.headers
    records: list[Json] = [cast(str, record["input_text"]) for record in logger.in_memory_records]
    return {
        "results": results,
        "calls": handler.calls,
        "records": records,
        "headers": None if stored is None else cast(Fields, redact_headers(stored)),
    }


async def _perform(logger: GalileoObserve, actions: list[Fields]) -> list[Json]:
    results: list[Json] = []
    ensure_headers = cast(
        Callable[[], Awaitable[bool]],
        logger._ensure_headers,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    ingest_request = cast(
        Callable[[], tuple[str, Fields] | None],
        logger._get_ingest_request,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    for action in actions:
        step = action["do"]
        if step == "push":
            logger.in_memory_records.append({"input_text": action["label"], "output_text": "out"})
            results.append(None)
        elif step == "flush":
            await logger.flush_in_memory_records()
            results.append(None)
        elif step == "health_check":
            results.append(cast(Fields, await logger.async_health_check()))
        elif step == "ensure_headers":
            results.append(await ensure_headers())
        elif step == "ingest_url":
            request = ingest_request()
            results.append(None if request is None else request[0])
        elif step == "redact":
            results.append(cast(Fields, redact_headers(cast(dict[str, str], action["headers"]))))
        else:
            message = f"unknown action: {step}"
            raise ValueError(message)
    return results
