import asyncio
import importlib
from collections.abc import AsyncGenerator, AsyncIterator, Awaitable, Callable, Sequence
from contextlib import aclosing
from typing import Final, Protocol, TypeAlias, TypedDict, TypeVar

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
    AgentServer,
    Recorder,
)
from litellm.a2a_protocol import LiteLLMSendMessageResponse as LiteLLMResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

import betterlite.a2a_protocol as our_package
from betterlite._native import NativeA2aClient, NativeA2aStream
from betterlite.a2a_protocol import client as client_module
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
    client_settings,
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
    role: str


class SendResponse(TypedDict):
    id: str
    jsonrpc: str
    result: MessageView


class StreamResponse(TypedDict):
    id: str
    jsonrpc: str
    result: MessageView


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


SEND_RESPONSE: Final[TypeAdapter[SendResponse]] = TypeAdapter(SendResponse)
STREAM_RESPONSE: Final[TypeAdapter[StreamResponse]] = TypeAdapter(StreamResponse)
# every answer main's guarded hands back, named so the spy below can match it exhaustively
NativeAnswer: TypeAlias = NativeA2aStream | NativeA2aClient | str | None
NATIVE_ANSWER: Final[TypeAdapter[NativeAnswer]] = TypeAdapter(
    NativeAnswer, config=ConfigDict(arbitrary_types_allowed=True)
)


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


class SettingsCalls:
    def __init__(self, *, forced_streaming: bool) -> None:
        self.forced_streaming = forced_streaming
        self.timeouts: list[float] = []
        self.requested: list[bool] = []

    def __call__(
        self,
        base_url: str,
        timeout: float,
        extra_headers: dict[str, str] | None,
        *,
        streaming: bool,
        relative_card_path: str | None,
    ) -> str:
        self.timeouts.append(timeout)
        self.requested.append(streaming)
        return client_settings(
            base_url,
            timeout,
            extra_headers,
            streaming=self.forced_streaming,
            relative_card_path=relative_card_path,
        )


_Answer = TypeVar("_Answer")


class NativeAnswers:
    def __init__(self) -> None:
        self.streams: list[NativeA2aStream] = []

    # guarded is generic over its answer, so the adapter names the union before the match
    async def record(self, call: Awaitable[_Answer]) -> _Answer:
        answer = await call
        match NATIVE_ANSWER.validate_python(answer):
            case NativeA2aStream() as stream:
                self.streams.append(stream)
            case NativeA2aClient() | str() | None:
                pass
        return answer


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


def collect_send_flow(
    answers: Sequence[BaseModel], recorder: Recorder, built: int
) -> SendFlow:
    results: list[dict[str, JsonValue]] = [
        JSON_OBJECT.validate_json(answer.model_dump_json(exclude_none=True))
        for answer in answers
    ]
    methods: list[str] = [entry["method"] for entry in recorder.calls]
    return SendFlow(built=built, methods=methods, results=results)


async def our_send_flow(server: AgentServer, recorder: Recorder) -> SendFlow:
    client = A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    built = len(recorder.calls)
    first = await client.send_message(our_request(REQUEST_ID))
    second = await client.send_message(our_request(SECOND_ID))
    return collect_send_flow([first, second], recorder, built)


async def their_send_flow(server: AgentServer, recorder: Recorder) -> SendFlow:
    client = THEIR_CLIENT.A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    built = len(recorder.calls)
    answers = [
        await client.send_message(their_request(request_id))
        for request_id in (REQUEST_ID, SECOND_ID)
    ]
    return collect_send_flow(answers, recorder, built)


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
    chunks: list[dict[str, JsonValue]] = [
        JSON_OBJECT.validate_json(chunk.model_dump_json(exclude_none=True))
        async for chunk in client.send_message_streaming(our_stream_request(STREAM_ID))
    ]
    return StreamFlow(methods=[entry["method"] for entry in recorder.calls], results=chunks)


async def their_stream_flow(server: AgentServer, recorder: Recorder) -> StreamFlow:
    client = THEIR_CLIENT.A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    chunks: list[dict[str, JsonValue]] = [
        JSON_OBJECT.validate_json(chunk.model_dump_json(exclude_none=True))
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


# the typed view of the one reply both sides must return, checked field by field
def check_reply_message(message: MessageView) -> None:
    assert message["kind"] == "message"
    assert message["role"] == "agent"
    assert message["messageId"] == MESSAGE_ID
    assert len(message["parts"]) == SINGLE
    part = message["parts"][0]
    assert part["kind"] == "text"
    assert part["text"] == REPLY_TEXT


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
    settings_calls = SettingsCalls(forced_streaming=False)
    original = main_module.client_settings
    main_module.client_settings = settings_calls
    try:
        with V1Server(Recorder()) as server:
            asyncio.run(timeout_flow(server))
    finally:
        main_module.client_settings = original
    assert settings_calls.timeouts == [
        DEFAULT_CLIENT_TIMEOUT,
        DEFAULT_CLIENT_TIMEOUT,
        DISTINCT_TIMEOUT,
        DISTINCT_TIMEOUT,
    ]


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
    # the complete dumps must match, so every field LiteLLM's client returns is compared
    assert ours["results"] == theirs["results"]
    responses = [SEND_RESPONSE.validate_python(result) for result in ours["results"]]
    assert [response["id"] for response in responses] == [REQUEST_ID, SECOND_ID]
    assert [response["jsonrpc"] for response in responses] == ["2.0", "2.0"]
    check_reply_message(responses[0]["result"])
    assert flow["our_posts"] == flow["their_posts"] == [
        EXPECTED_POST_HEADERS,
        EXPECTED_POST_HEADERS,
    ]


def test_stream_matches_litellm() -> None:
    flow = asyncio.run(both_streams())
    ours = flow["ours"]
    theirs = flow["theirs"]
    assert tuple(ours["methods"]) == tuple(theirs["methods"]) == STREAM_METHODS
    # the complete dumps must match, so every field LiteLLM's client yields is compared
    assert ours["results"] == theirs["results"]
    assert len(ours["results"]) == SINGLE
    chunk = STREAM_RESPONSE.validate_python(ours["results"][0])
    assert chunk["id"] == STREAM_ID
    assert chunk["jsonrpc"] == "2.0"
    check_reply_message(chunk["result"])
    assert flow["our_posts"] == flow["their_posts"] == [EXPECTED_POST_HEADERS]


async def close_class_stream_early(server: AgentServer, answers: NativeAnswers) -> bool:
    client = A2AClient(server.url, extra_headers=EXTRA_HEADERS)
    stream = client.send_message_streaming(our_stream_request(STREAM_ID))
    chunks: list[SendStreamingMessageResponse] = []
    async with aclosing(stream):
        async for chunk in stream:
            chunks.append(chunk)
            break
    assert len(chunks) == SINGLE
    assert chunks[0].id == STREAM_ID
    assert len(answers.streams) == SINGLE
    # the close reaches the native stream before the outer aclose returns
    remaining = await answers.streams[0].anext_chunk()
    return remaining is None


def test_stream_closes_early() -> None:
    answers = NativeAnswers()
    recorder = Recorder()
    settings_calls = SettingsCalls(forced_streaming=True)
    original_settings = main_module.client_settings
    original_guarded = main_module.guarded
    main_module.client_settings = settings_calls
    main_module.guarded = answers.record
    try:
        with V1Server(recorder) as server:
            closed = asyncio.run(close_class_stream_early(server, answers))
    finally:
        main_module.client_settings = original_settings
        main_module.guarded = original_guarded
    posts = [entry for entry in recorder.calls if entry["method"] == "POST"]
    assert settings_calls.requested == [False]
    # the forced settings have to put the native client on SSE, or no event is left pending
    assert len(posts) == SINGLE
    assert posts[0]["headers"].get("accept") == "text/event-stream"
    assert closed


class InnerCleanupError(Exception): ...


class RecordingStream:
    def __init__(self) -> None:
        self.ids: list[str | int] = []
        self.closed = 0

    async def chunks(
        self, a2a_client: NativeA2aClient, request: OurStreamRequest
    ) -> AsyncGenerator[SendStreamingMessageResponse, None]:
        _ = a2a_client
        self.ids.append(request.id)
        try:
            yield SendStreamingMessageResponse(id=request.id)
        finally:
            self.closed = self.closed + 1


class FailingStream:
    async def chunks(
        self, a2a_client: NativeA2aClient, request: OurStreamRequest
    ) -> AsyncGenerator[SendStreamingMessageResponse, None]:
        _ = a2a_client
        try:
            yield SendStreamingMessageResponse(id=request.id)
        finally:
            raise InnerCleanupError


async def close_stream_after_first_event(client: A2AClient, recorded: RecordingStream) -> None:
    stream = client.send_message_streaming(our_stream_request(STREAM_ID))
    async with aclosing(stream):
        async for chunk in stream:
            assert chunk.id == STREAM_ID
            break
    # the inner close runs inside aclose itself, not on a finalizer turn after it
    assert recorded.closed == SINGLE
    assert recorded.ids == [STREAM_ID]


async def close_stream_expect_error(client: A2AClient) -> None:
    stream = client.send_message_streaming(our_stream_request(STREAM_ID))
    with pytest.raises(InnerCleanupError):
        async with aclosing(stream):
            async for chunk in stream:
                assert chunk.id == STREAM_ID
                break


def test_stream_close_runs_cleanup(monkeypatch: pytest.MonkeyPatch) -> None:
    recorded = RecordingStream()
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(client_module, "asend_message_streaming", recorded.chunks)
    with V1Server(Recorder()) as server:
        asyncio.run(close_stream_after_first_event(A2AClient(server.url), recorded))
    assert recorded.closed == SINGLE


def test_stream_close_raises_error(monkeypatch: pytest.MonkeyPatch) -> None:
    failing = FailingStream()
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(client_module, "asend_message_streaming", failing.chunks)
    with V1Server(Recorder()) as server:
        asyncio.run(close_stream_expect_error(A2AClient(server.url)))
