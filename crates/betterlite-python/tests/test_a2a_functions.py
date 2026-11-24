import asyncio
import copy
from collections.abc import AsyncIterator, Awaitable, Coroutine
from typing import Final, Protocol, TypedDict

import pytest
from a2a.client import Client
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
from a2a_server import AGENT_NAME, AgentServer, Recorder
from litellm.a2a_protocol import LiteLLMSendMessageResponse as LiteLLMResponse
from litellm.a2a_protocol import card_resolver as litellm_cards
from litellm.a2a_protocol import main as litellm_main
from pydantic import BaseModel, JsonValue, TypeAdapter
from typing_extensions import override

from betterlite.a2a_protocol import card_resolver, main
from betterlite.a2a_protocol.errors import UnsupportedA2AArgument
from betterlite.a2a_protocol.types import LiteLLMSendMessageResponse
from betterlite.a2a_protocol.types import SendMessageRequest as OursRequest
from betterlite.a2a_protocol.types import SendStreamingMessageRequest as OursStreamRequest

REQUEST_ID = "req-1"
STREAM_ID = "req-2"
USER_MESSAGE_ID = "user-message"
USER_TEXT = "hi"
AGENT_ID = "agent-1"
EXTRA_HEADER = "x-agent-extra"
EXTRA_VALUE = "extra-value"
TRACE_HEADER = "x-litellm-trace-id"
AGENT_HEADER = "x-litellm-agent-id"
STREAM_EVENTS = 3
CARD_GETS = 2
SENDS = 2
SINGLE = 1
STATUS_EVENT: Final = (
    '{"id": "req-2", "jsonrpc": "2.0", "result": {"kind": "status-update", "taskId": "task-1",'
    ' "contextId": "context-1", "status": {"state": "working"}, "final": false}}'
)

CARD: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])

# LiteLLM's @client decorator hides asend_message's signature, so the calls go through a protocol
class LiteLLMSendCall(Protocol):
    def __call__(
        self,
        a2a_client: Client | None = ...,
        request: SendMessageRequest | None = ...,
        api_base: str | None = ...,
        *,
        agent_id: str | None = ...,
        agent_extra_headers: dict[str, str] | None = ...,
    ) -> Awaitable[LiteLLMResponse]: ...


class LiteLLMStreamCall(Protocol):
    def __call__(
        self,
        a2a_client: Client | None = ...,
        request: SendStreamingMessageRequest | None = ...,
    ) -> AsyncIterator[BaseModel]: ...


LITELLM_SEND: Final[LiteLLMSendCall] = litellm_main.asend_message
LITELLM_STREAM: Final[LiteLLMStreamCall] = litellm_main.asend_message_streaming


class InterfaceView(TypedDict):
    protocolBinding: str
    protocolVersion: str


INTERFACES: Final[TypeAdapter[list[InterfaceView]]] = TypeAdapter(list[InterfaceView])


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


def dumped(model: BaseModel) -> dict[str, JsonValue]:
    values: dict[str, JsonValue] = model.model_dump(mode="json", exclude_none=True)
    return values


def parsed_card(card: dict[str, JsonValue]) -> AgentCard:
    card_copy: dict[str, JsonValue] = copy.deepcopy(card)
    return parse_agent_card(card_copy)


async def fetched_cards(server: AgentServer) -> tuple[dict[str, JsonValue], AgentCard]:
    ours = await main.aget_agent_card(server.url)
    # LiteLLM annotates the compat AgentCard here but its resolver returns the proto
    theirs: AgentCard = await litellm_main.aget_agent_card(server.url)  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/main.py#L848
    return dumped(ours), theirs


async def our_client_card(server: AgentServer) -> dict[str, JsonValue]:
    client = await main.create_a2a_client(server.url)
    return CARD.validate_json(client.agent_card())


async def card_headers(server: AgentServer) -> None:
    headers: dict[str, str] = {EXTRA_HEADER: EXTRA_VALUE}
    card = await main.aget_agent_card(server.url, extra_headers=headers)
    client = await main.create_a2a_client(server.url, extra_headers=headers)
    assert dumped(card)["name"] == AGENT_NAME
    assert CARD.validate_json(client.agent_card())["name"] == AGENT_NAME


def test_card_matches_litellm() -> None:
    with V1Server(Recorder()) as server:
        our_card, their_card = asyncio.run(fetched_cards(server))
    assert parsed_card(our_card) == their_card


def test_client_card_matches() -> None:
    with V1Server(Recorder()) as server:
        our_card = asyncio.run(our_client_card(server))
        their_client = asyncio.run(litellm_main.create_a2a_client(server.url))
        reference: AgentCard = their_client._litellm_agent_card  # type: ignore[attr-defined]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/main.py#L841
    assert parsed_card(our_card) == reference


def test_card_headers_reach_agent() -> None:
    recorder = Recorder()
    with V1Server(recorder) as server:
        asyncio.run(card_headers(server))
    gets = [entry for entry in recorder.calls if entry["method"] == "GET"]
    assert len(gets) == CARD_GETS
    assert all(entry["headers"].get(EXTRA_HEADER) == EXTRA_VALUE for entry in gets)


def test_card_helpers_match() -> None:
    pair = (litellm_cards.AGENT_CARD_WELL_KNOWN_PATH, litellm_cards.PREV_AGENT_CARD_WELL_KNOWN_PATH)  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/a2a_protocol/card_resolver.py#L20-L30
    assert pair[0] == card_resolver.AGENT_CARD_WELL_KNOWN_PATH
    assert pair[1] == card_resolver.PREV_AGENT_CARD_WELL_KNOWN_PATH
    assert card_resolver.FOUNDRY_AGENT_CARD_PATH == litellm_cards.FOUNDRY_AGENT_CARD_PATH
    for url in (None, "", "http://localhost:9/", "http://127.0.0.1:9/", "https://agent.example/"):
        assert card_resolver.is_localhost_or_internal_url(url) == (
            litellm_cards.is_localhost_or_internal_url(url)
        )
    card: dict[str, JsonValue] = {
        "name": "agent",
        "url": "http://localhost:9/",
        "supportedInterfaces": [
            {"url": "http://localhost:9/", "protocolBinding": "jsonrpc", "protocolVersion": "1.0"}
        ],
    }
    normalized = card_resolver.normalize_agent_card_interfaces(card)
    ours_interface = INTERFACES.validate_python(normalized["supportedInterfaces"])[0]
    assert ours_interface["protocolBinding"] == "JSONRPC"
    assert ours_interface["protocolVersion"] == "0.3"
    reference = litellm_cards.normalize_agent_card_interfaces(parsed_card(card))
    assert parsed_card(normalized) == reference
    assert card_resolver.get_agent_card_url(card) == litellm_cards.get_agent_card_url(
        parsed_card(card)
    )
    interface: dict[str, JsonValue] = {"url": "https://first.example/"}
    interface_card: dict[str, JsonValue] = {"supportedInterfaces": [interface]}
    assert card_resolver.get_agent_card_url(interface_card) == litellm_cards.get_agent_card_url(
        parsed_card(interface_card)
    )
    moved = card_resolver.set_agent_card_url(card, "https://other.example/")
    assert card_resolver.get_agent_card_url(moved) == "https://other.example/"
    their_moved = parsed_card(card)
    litellm_cards.set_agent_card_url(their_moved, "https://other.example/")
    assert litellm_cards.get_agent_card_url(their_moved) == "https://other.example/"
    fixed = card_resolver.fix_agent_card_url(card, "https://agent.example")
    assert card_resolver.get_agent_card_url(fixed) == "https://agent.example/"
    their_fixed = litellm_cards.fix_agent_card_url(parsed_card(card), "https://agent.example")
    assert litellm_cards.get_agent_card_url(their_fixed) == "https://agent.example/"


def test_card_foreign_client() -> None:
    with pytest.raises(UnsupportedA2AArgument) as caught:
        asyncio.run(main.asend_message(a2a_client=object()))
    assert caught.value.argument == "a2a_client"


def test_card_foreign_request() -> None:
    with pytest.raises(UnsupportedA2AArgument) as caught:
        asyncio.run(main.asend_message(request=object()))
    assert caught.value.argument == "request"


async def litellm_send(
    request_id: str,
    *,
    client: Client | None = None,
    api_base: str | None = None,
    agent_id: str | None = None,
    headers: dict[str, str] | None = None,
) -> dict[str, JsonValue]:
    theirs: LiteLLMResponse = await LITELLM_SEND(
        a2a_client=client,
        request=SendMessageRequest(id=request_id, params=sdk_params()),
        api_base=api_base,
        agent_id=agent_id,
        agent_extra_headers=headers,
    )
    return dumped(theirs)


async def client_sends(
    server: AgentServer, request_id: str
) -> tuple[dict[str, JsonValue], dict[str, JsonValue]]:
    our_client = await main.create_a2a_client(server.url)
    ours = await main.asend_message(
        a2a_client=our_client, request=OursRequest(id=request_id, params=params())
    )
    their_client = await litellm_main.create_a2a_client(server.url)
    theirs = await litellm_send(request_id, client=their_client)
    return dumped(ours), theirs


async def direct_sends(
    server: AgentServer, request_id: str
) -> tuple[dict[str, JsonValue], dict[str, JsonValue]]:
    headers: dict[str, str] = {EXTRA_HEADER: EXTRA_VALUE}
    ours = await main.asend_message(
        request=OursRequest(id=request_id, params=params()),
        api_base=server.url,
        agent_id=AGENT_ID,
        agent_extra_headers=headers,
    )
    theirs = await litellm_send(
        request_id, api_base=server.url, agent_id=AGENT_ID, headers=headers
    )
    return dumped(ours), theirs


async def dict_sends(
    server: AgentServer, request_id: str
) -> tuple[dict[str, JsonValue], dict[str, JsonValue]]:
    call: dict[str, JsonValue] = {"id": request_id, "params": params()}
    plain = await main.asend_message(api_base=server.url, request=call)
    typed_request = OursRequest(id=request_id, params=params())
    typed = await main.asend_message(api_base=server.url, request=typed_request)
    return dumped(plain), dumped(typed)


async def ours_response(server: AgentServer, request_id: str) -> LiteLLMSendMessageResponse:
    client = await main.create_a2a_client(server.url)
    return await main.asend_message(
        a2a_client=client, request=OursRequest(id=request_id, params=params())
    )


async def looped_send(server: AgentServer, request_id: str) -> LiteLLMSendMessageResponse:
    client = await main.create_a2a_client(server.url)
    answer = main.send_message(client, OursRequest(id=request_id, params=params()))
    match answer:
        case Coroutine():
            return await answer
        case LiteLLMSendMessageResponse():
            return answer


def test_send_matches_litellm() -> None:
    with V1Server(Recorder()) as server:
        ours, theirs = asyncio.run(client_sends(server, REQUEST_ID))
    assert ours == theirs
    assert ours["id"] == REQUEST_ID


def test_api_base_send_matches() -> None:
    recorder = Recorder()
    with V1Server(recorder) as server:
        ours, theirs = asyncio.run(direct_sends(server, REQUEST_ID))
    assert ours == theirs
    posts = [entry for entry in recorder.calls if entry["method"] == "POST"]
    assert len(posts) == SENDS
    assert all(entry["headers"].get(TRACE_HEADER) for entry in posts)
    assert all(entry["headers"].get(AGENT_HEADER) == AGENT_ID for entry in posts)
    assert all(entry["headers"].get(EXTRA_HEADER) == EXTRA_VALUE for entry in posts)


def test_send_accepts_plain_dict() -> None:
    with V1Server(Recorder()) as server:
        plain, typed = asyncio.run(dict_sends(server, REQUEST_ID))
    assert plain == typed
    assert plain["id"] == REQUEST_ID


def test_send_hidden_params_empty() -> None:
    with V1Server(Recorder()) as server:
        response = asyncio.run(ours_response(server, REQUEST_ID))
    assert "_hidden_params" not in dumped(response)


def test_send_from_dict_matches() -> None:
    cases: list[tuple[dict[str, JsonValue], str | int | None]] = [
        ({"id": True}, None),
        ({"id": 7}, None),
        ({"id": "req"}, None),
        ({}, "given"),
        ({}, 9),
    ]
    for data, request_id in cases:
        ours = LiteLLMSendMessageResponse.from_dict(data, request_id)
        theirs = LiteLLMResponse.from_dict(data, request_id)
        assert dumped(ours) == dumped(theirs)


def test_send_from_dict_hidden() -> None:
    data: dict[str, JsonValue] = {"id": "req", "_hidden_params": {"provider": "a2a"}}
    ours = LiteLLMSendMessageResponse.from_dict(data)
    theirs = LiteLLMResponse.from_dict(data)
    assert dumped(ours) == dumped(theirs)
    assert dumped(ours)["_hidden_params"] == {"provider": "a2a"}


def test_send_inside_running_loop() -> None:
    with V1Server(Recorder()) as server:
        response = asyncio.run(looped_send(server, REQUEST_ID))
    assert dumped(response)["id"] == REQUEST_ID


def test_send_outside_running_loop() -> None:
    with V1Server(Recorder()) as server:
        client = asyncio.run(main.create_a2a_client(server.url))
        answer = main.send_message(client, OursRequest(id=REQUEST_ID, params=params()))
    match answer:
        case LiteLLMSendMessageResponse():
            assert dumped(answer)["id"] == REQUEST_ID
        case Coroutine():
            raise AssertionError("send_message returned a coroutine with no running loop")


async def both_streams(
    server: AgentServer, request_id: str
) -> tuple[list[dict[str, JsonValue]], list[dict[str, JsonValue]]]:
    our_client = await main.create_a2a_client(server.url, streaming=True)
    our_request = OursStreamRequest(id=request_id, params=params())
    ours = [
        dumped(chunk)
        async for chunk in main.asend_message_streaming(a2a_client=our_client, request=our_request)
    ]
    their_client = await litellm_main.create_a2a_client(server.url, streaming=True)
    theirs = [
        dumped(chunk)
        async for chunk in LITELLM_STREAM(
            a2a_client=their_client,
            request=SendStreamingMessageRequest(id=request_id, params=sdk_params()),
        )
    ]
    return ours, theirs


class FakeStream:
    def __init__(self, chunks: list[str]) -> None:
        self.chunks = chunks
        self.closed = 0

    async def anext_chunk(self) -> str | None:
        if not self.chunks:
            return None
        return self.chunks.pop(0)

    async def aclose(self) -> None:
        self.chunks.clear()
        self.closed = self.closed + 1


class FakeStreamer:
    def __init__(self, stream: FakeStream) -> None:
        self.stream = stream

    async def __call__(self, call: str) -> FakeStream:
        _ = call
        return self.stream


async def first_event() -> list[str]:
    request = OursStreamRequest(id=STREAM_ID, params=params())
    seen: list[str] = []
    async for chunk in main.asend_message_streaming(request=request):
        seen.append(chunk.model_dump_json())
        break
    return seen


def test_stream_matches_litellm() -> None:
    with V1Server(Recorder()) as server:
        ours, theirs = asyncio.run(both_streams(server, STREAM_ID))
    assert len(ours) == STREAM_EVENTS
    assert ours == theirs


def test_stream_closes_early(monkeypatch: pytest.MonkeyPatch) -> None:
    fake = FakeStream([STATUS_EVENT, STATUS_EVENT])
    # ast-grep-ignore: no-string-mocking
    monkeypatch.setattr(main, "send_message_streaming_direct", FakeStreamer(fake))
    seen = asyncio.run(first_event())
    assert len(seen) == SINGLE
    assert fake.closed == SINGLE
    assert fake.chunks == []
