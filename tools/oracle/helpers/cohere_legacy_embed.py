import asyncio
import json
from typing import TYPE_CHECKING, NotRequired, TypeAlias, TypedDict, cast
from unittest.mock import patch

import httpx
from litellm.llms.cohere.embed.handler import embedding  # pyright: ignore[reportUnknownVariableType]
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.utils import EmbeddingResponse, Usage

if TYPE_CHECKING:
    from collections.abc import Callable, Coroutine

    from litellm.litellm_core_utils.litellm_logging import Logging
    from litellm.litellm_core_utils.tokenizer import Encoding as Tokenizer

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

# litellm.embedding defaults to ten minutes, and the handler ignores it with a supplied client
TIMEOUT = 600.0
EMBEDDING = cast(
    "Callable[..., EmbeddingResponse | Coroutine[object, object, EmbeddingResponse]]",
    embedding,
)


class Run(TypedDict):
    status: int
    body: str
    data: NotRequired[Object | None]


class Call(TypedDict):
    model: str
    runs: list[Run]
    input: NotRequired[list[str]]
    optional_params: NotRequired[Object]
    headers: NotRequired[Object]
    api_key: NotRequired[str | None]


class Recorder:
    def __init__(self) -> None:
        self.records: list[Json] = []

    def pre_call(self, **params: Json) -> None:
        self.records.append({"phase": "pre_call", **params})

    def post_call(self, **params: Json) -> None:
        self.records.append({"phase": "post_call", **params})


class Encoding:
    def encode(self, value: str, /) -> list[int]:
        return list(range(len(value)))


def unused_transport(request: httpx.Request) -> httpx.Response:
    raise RuntimeError(f"the fixture answers no {request.method} request")


class ReplyTransport:
    def __init__(self, run: Run) -> None:
        self.status = run["status"]
        self.body = run["body"]

    async def post(
        self, url: str, data: str | None = None, headers: dict[str, str] | None = None
    ) -> httpx.Response:
        del headers
        self.data = cast("Json", json.loads(data or "{}"))
        self.url = url
        response = httpx.Response(
            self.status, content=self.body.encode("utf-8"), request=httpx.Request("POST", url)
        )
        response.raise_for_status()
        return response


def scenario(call: Call, run: Run) -> Object:
    transport = ReplyTransport(run)
    logging_obj = Recorder()
    outcome: Object
    try:
        handler = AsyncHTTPHandler(transport=httpx.MockTransport(unused_transport))
        with patch.object(handler, "post", transport.post):
            pending = cast(
                "Coroutine[object, object, EmbeddingResponse]",
                EMBEDDING(
                    model=call["model"],
                    input=list(call.get("input", [])),
                    model_response=EmbeddingResponse(),
                    logging_obj=cast("Logging", logging_obj),
                    optional_params=dict(call.get("optional_params", {})),
                    headers=cast("dict[str, str]", dict(call.get("headers", {}))),
                    encoding=cast("Tokenizer | None", Encoding()),
                    data=run.get("data"),
                    api_key=call.get("api_key"),
                    aembedding=True,
                    timeout=TIMEOUT,
                    client=handler,
                ),
            )
            response = asyncio.run(pending)
    except httpx.HTTPStatusError as error:
        outcome = {"ok": False, "status": error.response.status_code}
    except Exception:  # noqa: BLE001 - the wire contract carries either the reply or the failure
        outcome = {"ok": False, "status": None}
    else:
        usage = cast("Usage", response.usage)
        details = usage.prompt_tokens_details
        outcome = {
            "ok": True,
            "value": {
                "object": response.object,
                "data": cast("Json", response.data),
                "model": response.model,
                "usage": [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens],
                "billed": None
                if details is None
                else [details.text_tokens, details.image_tokens],
            },
        }
    return {"hooks": cast("Json", logging_obj.records), "outcome": outcome}


def run(input_value: Object) -> Json:
    call = cast("Call", input_value)
    return {"runs": [scenario(call, one) for one in call["runs"]]}
