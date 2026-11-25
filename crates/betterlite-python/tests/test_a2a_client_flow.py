import asyncio
import copy
import importlib
from collections.abc import AsyncGenerator, AsyncIterator, Awaitable, Callable
from typing import Final, Protocol, TypedDict

import litellm.a2a_protocol as their_package
import pytest
from a2a.client.card_resolver import parse_agent_card
from a2a.compat.v0_3.types import (
    Message,
    MessageSendParams,
    Part,
    Role,
    SendMessageRequest,
    SendStreamingMessageRequest,
    TextPart,
)
from a2a.types.a2a_pb2 import AgentCard
from a2a_server import (
    AGENT_NAME,
    CARD_PATH,
    MESSAGE_ID,
    REPLY_TEXT,
    TASK_ID,
    AgentServer,
    Recorder,
)
from litellm.a2a_protocol import LiteLLMSendMessageResponse as LiteLLMResponse
from pydantic import BaseModel, JsonValue, TypeAdapter
from typing_extensions import override

import betterlite.a2a_protocol as our_package
from betterlite.a2a_protocol import client as client_module
from betterlite.a2a_protocol.client import A2AClient
from betterlite.a2a_protocol.exceptions import (
    A2AAgentCardDiscoveryError,
    A2AAgentCardError,
    A2AConnectionError,
    A2AError,
    A2ALocalhostURLError,
)
from betterlite.a2a_protocol.main import (
    A2AClientType,
    aget_agent_card,
    asend_message,
    asend_message_streaming,
    create_a2a_client,
    send_message,
)
from betterlite.a2a_protocol.types import (
    LiteLLMSendMessageResponse,
    SendMessageRequest as OurRequest,
    SendStreamingMessageRequest as OurStreamRequest,
    SendStreamingMessageResponse,
)

REQUEST_ID: Final = "req-1"
SECOND_ID: Final = "req-2"
STREAM_ID: Final = "req-3"
USER_MESSAGE_ID: Final = "user-message"
USER_TEXT: Final = "hi"
EXTRA_HEADER: Final = "x-agent-extra"
EXTRA_VALUE: Final = "extra-value"
PLACEHOLDER_URL: Final = "http://127.0.0.1:1"
SINGLE: Final = 1
CLOSE_POLLS: Final = 50
DEFAULT_CLIENT_TIMEOUT: Final = 60.0
SEND_METHODS: Final[tuple[str, ...]] = ("GET", "POST", "POST")
STREAM_METHODS: Final[tuple[str, ...]] = ("GET", "POST")
# the sdk factory sets A2A-Version on its httpx client, and the native transport sets the same
EXPECTED_POST_HEADERS: Final[dict[str, str]] = {
    "a2a-version": "1.0",
    "content-type": "application/json",
    EXTRA_HEADER: EXTRA_VALUE,
}
POST_HEADER_NAMES: Final[tuple[str, ...]] = ("a2a-version", "content-type", EXTRA_HEADER)
EXTRA_HEADERS: Final[dict[str, str]] = {EXTRA_HEADER: EXTRA_VALUE}
EXPECTED_EXPORTS: Final[tuple[str, ...]] = (
    "A2AAgentCardError",
    "A2AClient",
    "A2AConnectionError",
    "A2AError",
    "A2ALocalhostURLError",
    "LiteLLMSendMessageResponse",
    "aget_agent_card",
    "asend_message",
    "asend_message_streaming",
    "create_a2a_client",
    "send_message",
)
JSON_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class CardFlow(TypedDict):
    ours: dict[str, JsonValue]
    theirs: AgentCard
    paths: list[str]
    headers: list[str | None]


class SendFlow(TypedDict):
    built: int
    methods: list[str]
    results: list[dict[str, JsonValue]]


class SendFlows(TypedDict):
    ours: SendFlow
    theirs: SendFlow
    our_posts: list[dict[str, str]]
    their_posts: list[dict[str, str]]


class StreamFlow(TypedDict):
    methods: list[str]
    results: list[dict[str, JsonValue]]


class StreamFlows(TypedDict):
    ours: StreamFlow
    theirs: StreamFlow
    our_posts: list[dict[str, str]]
    their_posts: list[dict[str, str]]


class TheirA2AClient(Protocol):
    async def get_agent_card(self) -> AgentCard: ...

    async def send_message(self, request: SendMessageRequest) -> LiteLLMResponse: ...

    def send_message_streaming(
        self, request: SendStreamingMessageRequest
    ) -> AsyncIterator[BaseModel]: ...


class TheirClientFactory(Protocol):
    def __call__(
        self,
        base_url: str,
        timeout: float = DEFAULT_CLIENT_TIMEOUT,
        extra_headers: dict[str, str] | None = None,
    ) -> TheirA2AClient: ...


class TheirClientModule(Protocol):
    def __getattr__(self, name: str) -> TheirClientFactory: ...


THEIR_CLIENT: Final[TheirClientModule] = importlib.import_module("litellm.a2a_protocol.client")


class V1Server(AgentServer):
    @override
    def card(self) -> dict[str, JsonValue]:
        card = super().card()
        card["supportedInterfaces"] = [
            {"url": self.base_url, "protocolBinding": "JSONRPC", "protocolVersion": "1.0"}
        ]
        return card


def dumped(model: BaseModel) -> dict[str, JsonValue]:
    fields: dict[str, JsonValue] = model.model_dump(mode="json", exclude_none=True)
    return fields


def params() -> dict[str, JsonValue]:
    message: dict[str, JsonValue] = {
        "role": "user",
        "parts": [{"kind": "text", "text": USER_TEXT}],
        "messageId": USER_MESSAGE_ID,
    }
    return {"message": message}


def sdk_params() -> MessageSendParams:
    part = Part(root=TextPart(text=USER_TEXT))
    message = Message(message_id=USER_MESSAGE_ID, parts=[part], role=Role.user)
    return MessageSendParams(message=message)


def our_request(request_id: str) -> OurRequest:
    fields = params()
    return OurRequest(id=request_id, params=fields)


def their_request(request_id: str) -> SendMessageRequest:
    fields = sdk_params()
    return SendMessageRequest(id=request_id, params=fields)


def our_stream_request(request_id: str) -> OurStreamRequest:
    fields = params()
    return OurStreamRequest(id=request_id, params=fields)


def their_stream_request(request_id: str) -> SendStreamingMessageRequest:
    fields = sdk_params()
    return SendStreamingMessageRequest(id=request_id, params=fields)


def methods_of(recorder: Recorder) -> list[str]:
    methods: list[str] = [entry["method"] for entry in recorder.calls]
    return methods


def paths_of(recorder: Recorder) -> list[str]:
    paths: list[str] = [entry["path"] for entry in recorder.calls]
    return paths


def headers_of(recorder: Recorder, name: str) -> list[str | None]:
    values: list[str | None] = [entry["headers"].get(name) for entry in recorder.calls]
    return values


def post_headers(recorder: Recorder) -> list[dict[str, str]]:
    selected: list[dict[str, str]] = [
        {name: entry["headers"][name] for name in POST_HEADER_NAMES if name in entry["headers"]}
        for entry in recorder.calls
        if entry["method"] == "POST"
    ]
    return selected


async def our_card(server: AgentServer) -> dict[str, JsonValue]:
    client = A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    card = await client.get_agent_card()
    return dumped(card)


async def their_card(server: AgentServer) -> AgentCard:
    client = THEIR_CLIENT.A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    card: AgentCard = await client.get_agent_card()
    return card


async def both_cards(server: AgentServer, recorder: Recorder) -> CardFlow:
    ours = await our_card(server)
    theirs = await their_card(server)
    return CardFlow(
        ours=ours,
        theirs=theirs,
        paths=paths_of(recorder),
        headers=headers_of(recorder, EXTRA_HEADER),
    )


async def our_send_flow(server: AgentServer, recorder: Recorder) -> SendFlow:
    client = A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    built = len(recorder.calls)
    first = await client.send_message(our_request(REQUEST_ID))
    second = await client.send_message(our_request(SECOND_ID))
    return SendFlow(
        built=built,
        methods=methods_of(recorder),
        results=[dumped(first), dumped(second)],
    )


async def their_send_flow(server: AgentServer, recorder: Recorder) -> SendFlow:
    client = THEIR_CLIENT.A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    built = len(recorder.calls)
    answered: list[BaseModel] = [
        await client.send_message(their_request(request_id))
        for request_id in (REQUEST_ID, SECOND_ID)
    ]
    return SendFlow(
        built=built,
        methods=methods_of(recorder),
        results=[dumped(model) for model in answered],
    )


async def both_sends() -> SendFlows:
    sides: dict[str, SendFlow] = {}
    posts: dict[str, list[dict[str, str]]] = {}
    flows: tuple[tuple[str, Callable[[AgentServer, Recorder], Awaitable[SendFlow]]], ...] = (
        ("ours", our_send_flow),
        ("theirs", their_send_flow),
    )
    for name, flow in flows:
        recorder = Recorder()
        with V1Server(recorder) as server:
            sides[name] = await flow(server, recorder)
        posts[name] = post_headers(recorder)
    return SendFlows(
        ours=sides["ours"],
        theirs=sides["theirs"],
        our_posts=posts["ours"],
        their_posts=posts["theirs"],
    )


async def our_stream_flow(server: AgentServer, recorder: Recorder) -> StreamFlow:
    client = A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    chunks = [
        dumped(chunk)
        async for chunk in client.send_message_streaming(our_stream_request(STREAM_ID))
    ]
    return StreamFlow(methods=methods_of(recorder), results=chunks)


async def their_stream_flow(server: AgentServer, recorder: Recorder) -> StreamFlow:
    client = THEIR_CLIENT.A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    chunks = [
        dumped(chunk)
        async for chunk in client.send_message_streaming(their_stream_request(STREAM_ID))
    ]
    return StreamFlow(methods=methods_of(recorder), results=chunks)


async def both_streams() -> StreamFlows:
    our_recorder = Recorder()
    their_recorder = Recorder()
    with V1Server(our_recorder) as our_server:
        ours = await our_stream_flow(our_server, our_recorder)
    with V1Server(their_recorder) as their_server:
        theirs = await their_stream_flow(their_server, their_recorder)
    return StreamFlows(
        ours=ours,
        theirs=theirs,
        our_posts=post_headers(our_recorder),
        their_posts=post_headers(their_recorder),
    )


def test_exports_match_litellm() -> None:
    assert tuple(our_package.__all__) == EXPECTED_EXPORTS
    assert our_package.__all__ == their_package.__all__
    assert our_package.A2AClient is A2AClient
    assert our_package.A2AAgentCardError is A2AAgentCardError
    assert our_package.A2AConnectionError is A2AConnectionError
    assert our_package.A2AError is A2AError
    assert our_package.A2ALocalhostURLError is A2ALocalhostURLError
    assert our_package.LiteLLMSendMessageResponse is LiteLLMSendMessageResponse
    assert our_package.aget_agent_card is aget_agent_card
    assert our_package.asend_message is asend_message
    assert our_package.asend_message_streaming is asend_message_streaming
    assert our_package.create_a2a_client is create_a2a_client
    assert our_package.send_message is send_message
    assert "A2AAgentCardDiscoveryError" not in tuple(our_package.__all__)
    assert issubclass(A2AAgentCardDiscoveryError, A2AAgentCardError)


def test_client_defaults() -> None:
    client = A2AClient(PLACEHOLDER_URL)
    assert client.base_url == PLACEHOLDER_URL
    assert client.timeout == DEFAULT_CLIENT_TIMEOUT
    assert client.extra_headers is None
    other = A2AClient(PLACEHOLDER_URL, DEFAULT_CLIENT_TIMEOUT, EXTRA_HEADERS)
    assert other.extra_headers == EXTRA_HEADERS


def test_card_matches_litellm() -> None:
    recorder = Recorder()
    with V1Server(recorder) as server:
        flow = asyncio.run(both_cards(server, recorder))
    assert parse_agent_card(copy.deepcopy(flow["ours"])) == flow["theirs"]
    assert flow["theirs"].name == AGENT_NAME
    assert flow["paths"] == [CARD_PATH, CARD_PATH]
    assert flow["headers"] == [EXTRA_VALUE, EXTRA_VALUE]


def test_send_matches_litellm() -> None:
    flow = asyncio.run(both_sends())
    ours = flow["ours"]
    theirs = flow["theirs"]
    assert ours["built"] == theirs["built"] == 0
    assert tuple(ours["methods"]) == tuple(theirs["methods"]) == SEND_METHODS
    assert ours["results"] == theirs["results"]
    assert [result["id"] for result in ours["results"]] == [REQUEST_ID, SECOND_ID]
    message = JSON_OBJECT.validate_python(ours["results"][0]["result"])
    assert message["kind"] == "message"
    assert message["parts"] == [{"kind": "text", "text": REPLY_TEXT}]
    assert flow["our_posts"] == flow["their_posts"] == [
        EXPECTED_POST_HEADERS,
        EXPECTED_POST_HEADERS,
    ]


def stream_message(chunk: dict[str, JsonValue]) -> None:
    result = JSON_OBJECT.validate_python(chunk["result"])
    assert chunk["id"] == STREAM_ID
    assert result["kind"] == "message"
    assert result["messageId"] == MESSAGE_ID


def test_stream_matches_litellm() -> None:
    flow = asyncio.run(both_streams())
    ours = flow["ours"]
    theirs = flow["theirs"]
    assert tuple(ours["methods"]) == tuple(theirs["methods"]) == STREAM_METHODS
    assert ours["results"] == theirs["results"]
    assert len(ours["results"]) == SINGLE
    stream_message(ours["results"][0])
    assert flow["our_posts"] == flow["their_posts"] == [EXPECTED_POST_HEADERS]


class FakeStreamingCall:
    def __init__(self, chunks: list[SendStreamingMessageResponse]) -> None:
        self.chunks = chunks
        self.sent = 0
        self.closed = 0

    async def __call__(
        self,
        *,
        a2a_client: A2AClientType | None = None,
        request: OurStreamRequest | None = None,
    ) -> AsyncIterator[SendStreamingMessageResponse]:
        _ = a2a_client
        _ = request
        try:
            for chunk in self.chunks:
                self.sent = self.sent + 1
                yield chunk
        finally:
            self.closed = self.closed + 1


def stream_chunk() -> SendStreamingMessageResponse:
    result: dict[str, JsonValue] = {"kind": "status-update", "taskId": TASK_ID}
    return SendStreamingMessageResponse(id=STREAM_ID, result=result)


async def early_close(
    server: AgentServer, fake: FakeStreamingCall
) -> tuple[list[dict[str, JsonValue]], int]:
    client = A2AClient(server.url)
    stream: AsyncGenerator[SendStreamingMessageResponse, None] = client.send_message_streaming(
        our_stream_request(STREAM_ID)
    )
    seen = [dumped(await anext(stream))]
    await stream.aclose()
    closed = 0
    for _ in range(CLOSE_POLLS):
        if fake.closed > 0:
            closed = fake.closed
            break
        await asyncio.sleep(0)
    return seen, closed


def test_stream_closes_early(monkeypatch: pytest.MonkeyPatch) -> None:
    fake = FakeStreamingCall([stream_chunk(), stream_chunk()])
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(client_module, "asend_message_streaming", fake)
    with V1Server(Recorder()) as server:
        seen, closed = asyncio.run(early_close(server, fake))
    assert len(seen) == SINGLE
    assert fake.sent == SINGLE
    assert closed == SINGLE
