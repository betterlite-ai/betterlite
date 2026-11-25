import asyncio
import importlib
from collections.abc import AsyncIterator, Awaitable, Callable
from typing import Final, Protocol, TypedDict

import litellm.a2a_protocol as their_package
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
    AgentServer,
    Recorder,
)
from litellm.a2a_protocol import LiteLLMSendMessageResponse as LiteLLMResponse
from pydantic import BaseModel, JsonValue, TypeAdapter
from typing_extensions import override

import betterlite.a2a_protocol as our_package
from betterlite._native import (
    NativeA2aClient,
    NativeA2aStream,
    create_client,
    get_agent_card,
    send_message_streaming_direct,
)
from betterlite.a2a_protocol import exceptions as a2a_exceptions
from betterlite.a2a_protocol import main as main_module
from betterlite.a2a_protocol.client import A2AClient
from betterlite.a2a_protocol.exceptions import (
    A2AAgentCardDiscoveryError,
    A2AAgentCardError,
    A2AConnectionError,
    A2AError,
    A2ALocalhostURLError,
)
from betterlite.a2a_protocol.main import (
    aget_agent_card,
    asend_message,
    asend_message_streaming,
    create_a2a_client,
    send_message,
)
from betterlite.a2a_protocol.types import (
    LiteLLMSendMessageResponse,
    SendStreamingMessageResponse,
)
from betterlite.a2a_protocol.types import (
    SendMessageRequest as OurRequest,
)
from betterlite.a2a_protocol.types import (
    SendStreamingMessageRequest as OurStreamRequest,
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
DEFAULT_CLIENT_TIMEOUT: Final = 60.0
DISTINCT_TIMEOUT: Final = 7.5
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


class TextPartView(TypedDict):
    kind: str
    text: str


class MessageView(TypedDict):
    kind: str
    messageId: str
    parts: list[TextPartView]


class MessageStub(TypedDict):
    kind: str
    messageId: str


class SendResponse(TypedDict):
    id: str
    result: MessageView


class StreamResponse(TypedDict):
    id: str
    result: MessageStub


class TimedSettings(TypedDict):
    timeout: float


class SendFlow(TypedDict):
    built: int
    methods: list[str]
    results: list[SendResponse]


class SendFlows(TypedDict):
    ours: SendFlow
    theirs: SendFlow
    our_posts: list[dict[str, str]]
    their_posts: list[dict[str, str]]


class StreamFlow(TypedDict):
    methods: list[str]
    results: list[StreamResponse]


class StreamFlows(TypedDict):
    ours: StreamFlow
    theirs: StreamFlow
    our_posts: list[dict[str, str]]
    their_posts: list[dict[str, str]]


SEND_RESPONSE: Final[TypeAdapter[SendResponse]] = TypeAdapter(SendResponse)
STREAM_RESPONSE: Final[TypeAdapter[StreamResponse]] = TypeAdapter(StreamResponse)
TIMED_SETTINGS: Final[TypeAdapter[TimedSettings]] = TypeAdapter(TimedSettings)


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


class CreateClientCalls:
    def __init__(self) -> None:
        self.timeouts: list[float] = []

    async def __call__(self, settings: str) -> NativeA2aClient:
        fields: TimedSettings = TIMED_SETTINGS.validate_json(settings)
        self.timeouts.append(fields["timeout"])
        return await create_client(settings)


class AgentCardCalls:
    def __init__(self) -> None:
        self.timeouts: list[float] = []

    async def __call__(self, settings: str) -> str:
        fields: TimedSettings = TIMED_SETTINGS.validate_json(settings)
        self.timeouts.append(fields["timeout"])
        return await get_agent_card(settings)


class DirectCalls:
    def __init__(self) -> None:
        self.streams: list[NativeA2aStream] = []

    async def __call__(self, call: str) -> NativeA2aStream:
        stream = await send_message_streaming_direct(call)
        self.streams.append(stream)
        return stream


class V1Server(AgentServer):
    @override
    def card(self) -> dict[str, JsonValue]:
        card = super().card()
        card["supportedInterfaces"] = [
            {"url": self.base_url, "protocolBinding": "JSONRPC", "protocolVersion": "1.0"}
        ]
        return card


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


async def our_card(server: AgentServer) -> dict[str, JsonValue]:
    client = A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    card = await client.get_agent_card()
    text = card.model_dump_json(exclude_none=True)
    fields: dict[str, JsonValue] = JSON_OBJECT.validate_json(text)
    return fields


async def their_card(server: AgentServer) -> AgentCard:
    client = THEIR_CLIENT.A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    card: AgentCard = await client.get_agent_card()
    return card


async def both_cards(server: AgentServer, recorder: Recorder) -> CardFlow:
    ours = await our_card(server)
    theirs = await their_card(server)
    paths: list[str] = [entry["path"] for entry in recorder.calls]
    headers: list[str | None] = [entry["headers"].get(EXTRA_HEADER) for entry in recorder.calls]
    return CardFlow(ours=ours, theirs=theirs, paths=paths, headers=headers)


async def our_send_flow(server: AgentServer, recorder: Recorder) -> SendFlow:
    client = A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    built = len(recorder.calls)
    first = await client.send_message(our_request(REQUEST_ID))
    second = await client.send_message(our_request(SECOND_ID))
    return SendFlow(
        built=built,
        methods=[entry["method"] for entry in recorder.calls],
        results=[
            SEND_RESPONSE.validate_json(first.model_dump_json(exclude_none=True)),
            SEND_RESPONSE.validate_json(second.model_dump_json(exclude_none=True)),
        ],
    )


async def their_send_flow(server: AgentServer, recorder: Recorder) -> SendFlow:
    client = THEIR_CLIENT.A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    built = len(recorder.calls)
    answers = [
        await client.send_message(their_request(request_id))
        for request_id in (REQUEST_ID, SECOND_ID)
    ]
    results: list[SendResponse] = []
    for answer in answers:
        results.append(SEND_RESPONSE.validate_json(answer.model_dump_json(exclude_none=True)))
    return SendFlow(
        built=built,
        methods=[entry["method"] for entry in recorder.calls],
        results=results,
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
        posts[name] = [
            {
                header: entry["headers"][header]
                for header in POST_HEADER_NAMES
                if header in entry["headers"]
            }
            for entry in recorder.calls
            if entry["method"] == "POST"
        ]
    return SendFlows(
        ours=sides["ours"],
        theirs=sides["theirs"],
        our_posts=posts["ours"],
        their_posts=posts["theirs"],
    )


async def our_stream_flow(server: AgentServer, recorder: Recorder) -> StreamFlow:
    client = A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    chunks = [
        STREAM_RESPONSE.validate_json(chunk.model_dump_json(exclude_none=True))
        async for chunk in client.send_message_streaming(our_stream_request(STREAM_ID))
    ]
    return StreamFlow(methods=[entry["method"] for entry in recorder.calls], results=chunks)


async def their_stream_flow(server: AgentServer, recorder: Recorder) -> StreamFlow:
    client = THEIR_CLIENT.A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    chunks = [
        STREAM_RESPONSE.validate_json(chunk.model_dump_json(exclude_none=True))
        async for chunk in client.send_message_streaming(their_stream_request(STREAM_ID))
    ]
    return StreamFlow(methods=[entry["method"] for entry in recorder.calls], results=chunks)


async def both_streams() -> StreamFlows:
    our_recorder = Recorder()
    their_recorder = Recorder()
    with V1Server(our_recorder) as our_server:
        ours = await our_stream_flow(our_server, our_recorder)
    with V1Server(their_recorder) as their_server:
        theirs = await their_stream_flow(their_server, their_recorder)
    our_posts: list[dict[str, str]] = [
        {
            header: entry["headers"][header]
            for header in POST_HEADER_NAMES
            if header in entry["headers"]
        }
        for entry in our_recorder.calls
        if entry["method"] == "POST"
    ]
    their_posts: list[dict[str, str]] = [
        {
            header: entry["headers"][header]
            for header in POST_HEADER_NAMES
            if header in entry["headers"]
        }
        for entry in their_recorder.calls
        if entry["method"] == "POST"
    ]
    return StreamFlows(ours=ours, theirs=theirs, our_posts=our_posts, their_posts=their_posts)


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
    assert a2a_exceptions.A2AAgentCardDiscoveryError is A2AAgentCardDiscoveryError


def test_client_defaults() -> None:
    client = A2AClient(PLACEHOLDER_URL)
    assert client.base_url == PLACEHOLDER_URL
    assert client.timeout == DEFAULT_CLIENT_TIMEOUT
    assert client.extra_headers is None
    other = A2AClient(PLACEHOLDER_URL, DEFAULT_CLIENT_TIMEOUT, EXTRA_HEADERS)
    assert other.extra_headers == EXTRA_HEADERS


async def timeout_flow(server: AgentServer) -> None:
    default_client = A2AClient(server.url)
    await default_client.get_agent_card()
    await default_client.send_message(our_request(REQUEST_ID))
    tuned_client = A2AClient(server.url, DISTINCT_TIMEOUT)
    await tuned_client.get_agent_card()
    await tuned_client.send_message(our_request(SECOND_ID))


def test_client_forwards_timeouts() -> None:
    create_calls = CreateClientCalls()
    card_calls = AgentCardCalls()
    original_create = main_module.create_client
    original_card = main_module.get_agent_card
    main_module.create_client = create_calls
    main_module.get_agent_card = card_calls
    try:
        with V1Server(Recorder()) as server:
            asyncio.run(timeout_flow(server))
    finally:
        main_module.create_client = original_create
        main_module.get_agent_card = original_card
    assert create_calls.timeouts == [DEFAULT_CLIENT_TIMEOUT, DISTINCT_TIMEOUT]
    assert card_calls.timeouts == [DEFAULT_CLIENT_TIMEOUT, DISTINCT_TIMEOUT]


def test_card_matches_litellm() -> None:
    recorder = Recorder()
    with V1Server(recorder) as server:
        flow = asyncio.run(both_cards(server, recorder))
    assert parse_agent_card(flow["ours"]) == flow["theirs"]
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
    message = ours["results"][0]["result"]
    assert message["kind"] == "message"
    assert message["messageId"] == MESSAGE_ID
    assert len(message["parts"]) == SINGLE
    part = message["parts"][0]
    assert part["kind"] == "text"
    assert part["text"] == REPLY_TEXT
    assert flow["our_posts"] == flow["their_posts"] == [
        EXPECTED_POST_HEADERS,
        EXPECTED_POST_HEADERS,
    ]


def test_stream_matches_litellm() -> None:
    flow = asyncio.run(both_streams())
    ours = flow["ours"]
    theirs = flow["theirs"]
    assert tuple(ours["methods"]) == tuple(theirs["methods"]) == STREAM_METHODS
    assert ours["results"] == theirs["results"]
    assert len(ours["results"]) == SINGLE
    chunk = ours["results"][0]
    assert chunk["id"] == STREAM_ID
    assert chunk["result"]["kind"] == "message"
    assert chunk["result"]["messageId"] == MESSAGE_ID
    assert flow["our_posts"] == flow["their_posts"] == [EXPECTED_POST_HEADERS]


async def first_chunk(
    held: list[AsyncIterator[SendStreamingMessageResponse]], server: AgentServer
) -> list[SendStreamingMessageResponse]:
    stream = asend_message_streaming(
        request=our_stream_request(STREAM_ID), api_base=server.url
    )
    held.append(stream)
    seen: list[SendStreamingMessageResponse] = []
    async for chunk in stream:
        seen.append(chunk)
        break
    return seen


async def remaining_chunks(stream: NativeA2aStream) -> list[str]:
    chunks: list[str] = []
    while True:
        chunk = await stream.anext_chunk()
        if chunk is None:
            return chunks
        chunks.append(chunk)


def test_stream_closes_early() -> None:
    direct_calls = DirectCalls()
    original = main_module.send_message_streaming_direct
    main_module.send_message_streaming_direct = direct_calls
    held: list[AsyncIterator[SendStreamingMessageResponse]] = []
    try:
        with V1Server(Recorder()) as server:
            seen = asyncio.run(first_chunk(held, server))
    finally:
        main_module.send_message_streaming_direct = original
    assert len(seen) == SINGLE
    assert len(held) == SINGLE
    assert len(direct_calls.streams) == SINGLE
    # the server sent further events, so a handle left open would hand them back here
    assert asyncio.run(remaining_chunks(direct_calls.streams[0])) == []
