import asyncio
import json
from collections.abc import Callable, Coroutine
from typing import TYPE_CHECKING, NotRequired, TypeAlias, TypedDict, cast
from unittest.mock import patch

import httpx
from litellm.llms.cohere.embed.handler import embedding  # pyright: ignore[reportUnknownVariableType]
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import EmbeddingResponse, Usage

if TYPE_CHECKING:
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


class Reply(TypedDict):
    status: int
    body: str


class Run(TypedDict):
    reply: Reply
    data: NotRequired[Object | None]
    complete_api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    headers: NotRequired[Object]


class Call(TypedDict):
    mode: str
    model: str
    runs: list[Run]
    input: NotRequired[list[str]]
    optional_params: NotRequired[Object]


class Invocation(TypedDict):
    model: str
    input: list[str]
    optional_params: Object
    headers: Object
    data: Object | None
    complete_api_base: str | None
    api_key: str | None


class Recorder:
    def __init__(self) -> None:
        self.records: list[Json] = []

    def pre_call(self, **params: Json) -> None:
        additional = params.get("additional_args")
        args: Object = additional if isinstance(additional, dict) else {}
        record: Object = {
            "phase": "pre_call",
            "input": params.get("input"),
            "api_key": params.get("api_key"),
            "body": args.get("complete_input_dict"),
        }
        if "headers" in args:
            record["headers"] = args.get("headers")
            record["api_base"] = args.get("api_base")
        self.records.append(record)

    def post_call(self, **params: Json) -> None:
        additional = params.get("additional_args")
        args: Object = additional if isinstance(additional, dict) else {}
        self.records.append(
            {
                "phase": "post_call",
                "input": params.get("input"),
                "api_key": params.get("api_key"),
                "body": args.get("complete_input_dict"),
                "original_response": params.get("original_response"),
            }
        )


class Encoding:
    def __init__(self) -> None:
        self.encoded: list[str] = []

    def encode(self, value: str, /) -> list[int]:
        self.encoded.append(value)
        return list(range(len(value)))


def unused_transport(request: httpx.Request) -> httpx.Response:
    raise RuntimeError(f"the fixture answers no {request.method} request")


class ReplyTransport:
    def __init__(self, reply: Reply) -> None:
        self.status = reply["status"]
        self.body = reply["body"]
        self.calls: list[Object] = []

    def record(self, url: str, data: str | None, headers: dict[str, str] | None) -> None:
        self.calls.append(
            {
                "url": url,
                "headers": cast("Json", headers or {}),
                "data": cast("Json", json.loads(data or "{}")),
            }
        )

    def answer(self, url: str) -> httpx.Response:
        response = httpx.Response(
            self.status,
            content=self.body.encode("utf-8"),
            request=httpx.Request("POST", url),
        )
        response.raise_for_status()
        return response

    def post(
        self, url: str, data: str | None = None, headers: dict[str, str] | None = None
    ) -> httpx.Response:
        self.record(url, data, headers)
        return self.answer(url)

    async def async_post(
        self, url: str, data: str | None = None, headers: dict[str, str] | None = None
    ) -> httpx.Response:
        self.record(url, data, headers)
        return self.answer(url)


def invocation_of(call: Call, run: Run) -> Invocation:
    return {
        "model": call["model"],
        "input": list(call.get("input", [])),
        "optional_params": dict(call.get("optional_params", {})),
        "headers": run.get("headers", {}),
        "data": run.get("data"),
        "complete_api_base": run.get("complete_api_base"),
        "api_key": run.get("api_key"),
    }


def call_handler(
    request: Invocation,
    client: HTTPHandler | AsyncHTTPHandler,
    logging_obj: Recorder,
    encoding: Encoding,
    *,
    is_async: bool,
) -> EmbeddingResponse | Coroutine[object, object, EmbeddingResponse]:
    return EMBEDDING(
        model=request["model"],
        input=request["input"],
        model_response=EmbeddingResponse(),
        logging_obj=cast("Logging", logging_obj),
        optional_params=request["optional_params"],
        headers=cast("dict[str, str]", request["headers"]),
        encoding=cast("Tokenizer | None", encoding),
        data=request["data"],
        complete_api_base=request["complete_api_base"],
        api_key=request["api_key"],
        aembedding=is_async,
        timeout=TIMEOUT,
        client=client,
    )


def status_of(error: Exception) -> Json:
    if isinstance(error, httpx.HTTPStatusError):
        return error.response.status_code
    return None


def outcome_of(response: EmbeddingResponse) -> Object:
    usage = cast("Usage", response.usage)
    details = usage.prompt_tokens_details
    return {
        "ok": True,
        "value": {
            "object": response.object,
            "data": cast("Json", response.data),
            "model": response.model,
            "usage": [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens],
            "billed": None if details is None else [details.text_tokens, details.image_tokens],
        },
    }


def scenario(call: Call, run: Run) -> Object:
    is_async = call["mode"] == "async"
    request = invocation_of(call, run)
    transport = ReplyTransport(run["reply"])
    logging_obj = Recorder()
    encoding = Encoding()
    outcome: Object
    try:
        if is_async:
            handler = AsyncHTTPHandler(transport=httpx.MockTransport(unused_transport))
            with patch.object(handler, "post", transport.async_post):
                pending = cast(
                    "Coroutine[object, object, EmbeddingResponse]",
                    call_handler(request, handler, logging_obj, encoding, is_async=is_async),
                )
                response = asyncio.run(pending)
        else:
            handler = HTTPHandler(
                client=httpx.Client(transport=httpx.MockTransport(unused_transport))
            )
            with patch.object(handler, "post", transport.post):
                response = cast(
                    "EmbeddingResponse",
                    call_handler(request, handler, logging_obj, encoding, is_async=is_async),
                )
    except Exception as error:  # noqa: BLE001 - the wire contract carries either the reply or the failure
        outcome = {"ok": False, "status": status_of(error)}
    else:
        outcome = outcome_of(response)
    record = transport.calls[0]
    return {
        "url": record["url"],
        "headers": record["headers"],
        "data": record["data"],
        "hooks": cast("Json", logging_obj.records),
        "outcome": outcome,
        "encoded": cast("Json", encoding.encoded),
    }


def run(input_value: Object) -> Json:
    call = cast("Call", input_value)
    if call["mode"] not in {"sync", "async"}:
        raise ValueError("mode must be sync or async")
    return {"runs": [scenario(call, one) for one in call["runs"]]}
