import asyncio
import json
from collections.abc import Callable, Coroutine
from datetime import UTC, datetime
from functools import partial
from typing import ClassVar, Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.embed import handler as embed_handler
from litellm.llms.azure_ai.embed.handler import AzureAIEmbedding
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import EmbeddingResponse
from openai import AsyncOpenAI, OpenAI
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

TEXT_BASE: Final = "https://text.test/v1"
AEMBEDDING: Final = True
# a fixed start time keeps the helper off the clock; Logging only stores it in its details
LOG_START: Final = datetime(1970, 1, 1, tzinfo=UTC)
BODY_KEYS: Final = frozenset({"model", "input"})
NONE_MESSAGES: Final = frozenset(
    {
        "/image/embeddings route returned None Embeddings.",
        "/v1/embeddings route returned None Embeddings.",
    }
)

JSON_OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
TIMEOUT: TypeAdapter[float] = TypeAdapter(float)


class HeaderCall(TypedDict):
    name: str
    value: str


class ReplyCall(TypedDict):
    status: int
    headers: list[HeaderCall]
    body: str


class FlowCall(TypedDict):
    mode: Literal["sync", "async"]
    model: str
    input: list[str]
    options: dict[str, JsonValue]
    api_base: str | None
    api_key: str | None
    timeout: int
    image_reply: ReplyCall
    text_reply: ReplyCall
    client: Literal["none", "openai", "handler"]


class Input(TypedDict):
    flows: list[FlowCall]


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ReplyView(TypedDict):
    object: str
    model: str | None
    data: list[JsonValue]
    usage: UsageView


class TextBody(TypedDict):
    model: str
    input: list[JsonValue]


class AsyncParams(TypedDict):
    timeout: float


REPLY: TypeAdapter[ReplyView] = TypeAdapter(ReplyView)
TEXT_BODY: TypeAdapter[TextBody] = TypeAdapter(TextBody)


Client: TypeAlias = HTTPHandler | AsyncHTTPHandler | OpenAI | AsyncOpenAI | None

EmbedCall = Callable[
    [
        str,
        list[str],
        float,
        Logging,
        EmbeddingResponse,
        dict[str, JsonValue],
        str | None,
        str | None,
        Client,
        bool | None,
    ],
    EmbeddingResponse,
]
ReusedEmbedCall = Callable[
    [
        str,
        list[str],
        float,
        Logging,
        EmbeddingResponse,
        dict[str, JsonValue],
        str | None,
        str | None,
        Client,
        bool | None,
    ],
    Coroutine[None, None, EmbeddingResponse],
]


# LiteLLM's embedding members carry no return type, so these protocols restate the calls we make
class SyncEmbeddingApi(Protocol):
    @property
    def embedding(self) -> EmbedCall: ...


class AsyncEmbeddingApi(Protocol):
    @property
    def embedding(self) -> ReusedEmbedCall: ...


class FlowState:
    def __init__(self) -> None:
        self.calls: list[JsonValue] = []
        self.constructed: list[JsonValue] = []


def reply_response(reply: ReplyCall, request: httpx.Request) -> httpx.Response:
    headers = {header["name"]: header["value"] for header in reply["headers"]}
    answer = httpx.Response(
        reply["status"],
        headers=headers,
        content=reply["body"],
        request=request,
    )
    return answer


class ImageTransport(httpx.MockTransport):
    def __init__(self, state: FlowState, reply: ReplyCall) -> None:
        super().__init__(self.respond)
        self.state = state
        self.reply = reply

    def respond(self, request: httpx.Request) -> httpx.Response:
        body = JSON_OBJECT.validate_json(request.content)
        call: dict[str, JsonValue] = {
            "kind": "image",
            "url": str(request.url),
            "authorization": request.headers.get("authorization"),
            "body": body,
        }
        self.state.calls.append(call)
        return reply_response(self.reply, request)


class TextTransport(httpx.MockTransport):
    def __init__(self, state: FlowState, reply: ReplyCall) -> None:
        super().__init__(self.respond)
        self.state = state
        self.reply = reply

    def respond(self, request: httpx.Request) -> httpx.Response:
        body = JSON_OBJECT.validate_json(request.content)
        text = TEXT_BODY.validate_json(request.content)
        options: dict[str, JsonValue] = {
            name: value for name, value in body.items() if name not in BODY_KEYS
        }
        call: dict[str, JsonValue] = {
            "kind": "text",
            "model": text["model"],
            "input": text["input"],
            "options": options,
        }
        self.state.calls.append(call)
        return reply_response(self.reply, request)


class RecordingHandler(HTTPHandler):
    recorder: ClassVar[FlowState]
    transport: ClassVar[httpx.MockTransport]

    @override
    def create_client(self) -> httpx.Client:
        created: dict[str, JsonValue] = {
            "kind": "sync",
            "timeout": TIMEOUT.validate_python(self.timeout),
        }
        RecordingHandler.recorder.constructed.append(created)
        return httpx.Client(transport=RecordingHandler.transport)


def build_async_client(
    recorder: FlowState,
    transport: httpx.MockTransport,
    llm_provider: str,
    params: AsyncParams,
) -> AsyncHTTPHandler:
    del llm_provider
    created: dict[str, JsonValue] = {"kind": "async", "timeout": params["timeout"]}
    recorder.constructed.append(created)
    return AsyncHTTPHandler(transport=transport, timeout=params["timeout"])


def build_client(flow: FlowCall, image: ImageTransport, text: TextTransport) -> Client:
    kind = flow["client"]
    if kind == "none":
        return None
    if kind == "handler":
        if flow["mode"] == "sync":
            client = httpx.Client(transport=image)
            return RecordingHandler(client=client)
        return AsyncHTTPHandler(transport=image)
    if flow["mode"] == "sync":
        return OpenAI(
            api_key="fixture-text-key",
            base_url=TEXT_BASE,
            http_client=httpx.Client(transport=text),
        )
    return AsyncOpenAI(
        api_key="fixture-text-key",
        base_url=TEXT_BASE,
        http_client=httpx.AsyncClient(transport=text),
    )


def call_sync(
    api: SyncEmbeddingApi, flow: FlowCall, client: Client, log: Logging
) -> EmbeddingResponse:
    return api.embedding(
        flow["model"],
        flow["input"],
        float(flow["timeout"]),
        log,
        EmbeddingResponse(),
        flow["options"],
        flow["api_key"],
        flow["api_base"],
        client,
        None,
    )


def call_async(
    api: AsyncEmbeddingApi, flow: FlowCall, client: Client, log: Logging
) -> Coroutine[None, None, EmbeddingResponse]:
    return api.embedding(
        flow["model"],
        flow["input"],
        float(flow["timeout"]),
        log,
        EmbeddingResponse(),
        flow["options"],
        flow["api_key"],
        flow["api_base"],
        client,
        AEMBEDDING,
    )


def flow_response(flow: FlowCall, client: Client) -> EmbeddingResponse:
    log = Logging(
        model=flow["model"],
        messages=None,
        stream=False,
        call_type="embedding",
        start_time=LOG_START,
        litellm_call_id="",
        function_id="embedding",
    )
    handler = AzureAIEmbedding()
    if flow["mode"] == "sync":
        sync: SyncEmbeddingApi = handler
        return call_sync(sync, flow, client, log)
    async_handler: AsyncEmbeddingApi = handler
    return asyncio.run(call_async(async_handler, flow, client, log))


def flow_outcome(flow: FlowCall) -> dict[str, JsonValue]:
    state = FlowState()
    image = ImageTransport(state, flow["image_reply"])
    text = TextTransport(state, flow["text_reply"])
    RecordingHandler.recorder = state
    RecordingHandler.transport = image
    embed_handler.get_async_httpx_client = partial(build_async_client, state, image)
    client = build_client(flow, image, text)
    failure: dict[str, JsonValue]
    try:
        response = flow_response(flow, client)
    except httpx.HTTPStatusError as error:
        failure = failure_outcome(state, "HTTPStatusError", error.response.status_code)
    except json.JSONDecodeError:
        failure = failure_outcome(state, "JSONDecodeError", None)
    except KeyError:
        failure = failure_outcome(state, "KeyError", None)
    except IndexError:
        failure = failure_outcome(state, "IndexError", None)
    except ValueError:
        failure = failure_outcome(state, "ValueError", None)
    except Exception as error:
        if str(error) not in NONE_MESSAGES:
            raise
        failure = failure_outcome(state, "Exception", None)
    else:
        return answer_outcome(state, response)
    return failure


def outcome(
    state: FlowState,
    response: dict[str, JsonValue] | None,
    error: dict[str, JsonValue] | None,
) -> dict[str, JsonValue]:
    record: dict[str, JsonValue] = {
        "calls": state.calls,
        "constructed": state.constructed,
        "response": response,
        "error": error,
    }
    return record


def failure_outcome(state: FlowState, kind: str, status: int | None) -> dict[str, JsonValue]:
    error: dict[str, JsonValue] = {"kind": kind, "status": status}
    return outcome(state, None, error)


def answer_outcome(state: FlowState, response: EmbeddingResponse) -> dict[str, JsonValue]:
    view = REPLY.validate_json(response.model_dump_json())
    usage: dict[str, JsonValue] = {
        "prompt_tokens": view["usage"]["prompt_tokens"],
        "completion_tokens": view["usage"]["completion_tokens"],
        "total_tokens": view["usage"]["total_tokens"],
    }
    answer: dict[str, JsonValue] = {
        "object": view["object"],
        "model": view["model"],
        "data": view["data"],
        "usage": usage,
    }
    return outcome(state, answer, None)


def run(input_value: Input) -> JsonValue:
    embed_handler.HTTPHandler = RecordingHandler
    flows: list[JsonValue] = [flow_outcome(flow) for flow in input_value["flows"]]
    answer: dict[str, JsonValue] = {"flows": flows}
    return answer
