import asyncio
import json
from typing import TypedDict

import pytest
from a2a_server import (
    AGENT_NAME,
    ARTIFACT_TEXT,
    CARD_PATH,
    DEAD_URL,
    LEGACY_CARD_PATH,
    LOCALHOST_URL,
    PROTOCOL_VERSION,
    REPLY_TEXT,
    SEND_METHOD,
    STREAM_METHOD,
    TASK_ID,
    AgentServer,
    RecordedCall,
    Recorder,
    call_of,
    header_of,
)
from pydantic import JsonValue, TypeAdapter

from betterlite import _native

REQUEST_ID = "req-1"
STREAM_REQUEST_ID = "req-2"
USER_MESSAGE_ID = "user-message"
TRACE_ID = "trace-1"
AGENT_ID = "agent-1"
EXTRA_HEADER = "X-Test"
EXTRA_VALUE = "yes"
TRACE_HEADER = "X-LiteLLM-Trace-Id"
AGENT_HEADER = "X-LiteLLM-Agent-Id"
AGENT_TRACE = "agent-trace"
ACCEPT_HEADER = "Accept"
EVENT_STREAM = "text/event-stream"
DEAD_STATUS = 503
LLM_PROVIDER = "a2a_agent"
STATUS_LABEL = "status-update"
ARTIFACT_LABEL = "artifact-update"
MESSAGE_LABEL = "message"
AGENT_ROLE = "agent"
WORKING_STATE = "working"
MISSING_CLIENT = "Either a2a_client or api_base is required for standard A2A flow"
SINGLE = 1
CARD_PROBES = 2
STREAM_EVENTS = 3


class TextPart(TypedDict):
    kind: str
    text: str


class AgentMessage(TypedDict):
    kind: str
    role: str
    parts: list[TextPart]
    messageId: str


class SendView(TypedDict):
    id: str
    jsonrpc: str
    result: AgentMessage


class TaskStatus(TypedDict):
    state: str


class StatusResult(TypedDict):
    kind: str
    taskId: str
    status: TaskStatus
    final: bool


class StatusChunk(TypedDict):
    result: StatusResult


class Artifact(TypedDict):
    artifactId: str
    parts: list[TextPart]


class ArtifactResult(TypedDict):
    kind: str
    taskId: str
    artifact: Artifact


class ArtifactChunk(TypedDict):
    result: ArtifactResult


class MessageChunk(TypedDict):
    result: AgentMessage


class Capabilities(TypedDict):
    streaming: bool


class CardView(TypedDict):
    protocolVersion: str
    name: str
    url: str
    capabilities: Capabilities


class Interface(TypedDict):
    url: str
    protocolBinding: str
    protocolVersion: str


class InterfaceCard(TypedDict):
    supportedInterfaces: list[Interface]


class UrlCard(TypedDict):
    url: str


class ErrorView(TypedDict):
    type: str
    message: str
    status_code: int | None
    llm_provider: str | None
    url: str | None


def settings_of(
    server: AgentServer,
    *,
    extra_headers: dict[str, JsonValue] | None = None,
    streaming: bool = False,
    relative_card_path: str | None = None,
) -> str:
    settings: dict[str, JsonValue] = {"base_url": server.url}
    if extra_headers is not None:
        settings["extra_headers"] = extra_headers
    if streaming:
        settings["streaming"] = True
    if relative_card_path is not None:
        settings["relative_card_path"] = relative_card_path
    return json.dumps(settings)


def request_of(request_id: str) -> dict[str, JsonValue]:
    message: dict[str, JsonValue] = {
        "role": "user",
        "parts": [{"kind": "text", "text": "hi"}],
        "messageId": USER_MESSAGE_ID,
    }
    return {"id": request_id, "params": {"message": message}}


def send_call(
    *,
    api_base: str | None = None,
    trace_id: str | None = None,
    agent_id: str | None = None,
    agent_extra_headers: dict[str, JsonValue] | None = None,
    request_id: str = REQUEST_ID,
) -> str:
    call: dict[str, JsonValue] = {"request": request_of(request_id)}
    if api_base is not None:
        call["api_base"] = api_base
    if trace_id is not None:
        call["litellm_trace_id"] = trace_id
    if agent_id is not None:
        call["agent_id"] = agent_id
    if agent_extra_headers is not None:
        call["agent_extra_headers"] = agent_extra_headers
    return json.dumps(call)


def recorded(recorder: Recorder, method: str) -> list[RecordedCall]:
    return [call for call in recorder.calls if call["method"] == method]


async def drain(stream: _native.NativeA2aStream) -> list[str]:
    chunks: list[str] = []
    while True:
        chunk = await stream.anext_chunk()
        if chunk is None:
            break
        chunks.append(chunk)
    return chunks


async def send_with(settings: str, call: str) -> str:
    client = await _native.create_client(settings)
    return await client.send_message(call)


async def failing_send(settings: str, call: str) -> None:
    client = await _native.create_client(settings)
    await client.send_message(call)


async def direct_reply(call: str) -> str:
    return await _native.send_message_direct(call)


async def failing_direct(call: str) -> None:
    await _native.send_message_direct(call)


async def card_for(settings: str) -> str:
    return await _native.get_agent_card(settings)


async def created_card(settings: str) -> tuple[str, str]:
    client = await _native.create_client(settings)
    return client.base_url, client.agent_card()


async def stream_with(settings: str, call: str) -> list[str]:
    client = await _native.create_client(settings)
    stream = await client.send_message_streaming(call)
    chunks = await drain(stream)
    await stream.aclose()
    await stream.aclose()
    assert await stream.anext_chunk() is None
    return chunks


async def first_then_close(settings: str, call: str) -> str:
    client = await _native.create_client(settings)
    stream = await client.send_message_streaming(call)
    first = await stream.anext_chunk()
    await stream.aclose()
    await stream.aclose()
    assert await stream.anext_chunk() is None
    assert first is not None
    return first


async def stream_direct(call: str) -> list[str]:
    stream = await _native.send_message_streaming_direct(call)
    return await drain(stream)


def test_send_message_returns_the_agent_reply() -> None:
    with AgentServer(Recorder()) as server:
        text = asyncio.run(send_with(settings_of(server), send_call()))
    payload = TypeAdapter(SendView).validate_json(text)
    unstructured = TypeAdapter(dict[str, JsonValue]).validate_json(text)
    assert payload["id"] == REQUEST_ID
    assert payload["jsonrpc"] == "2.0"
    assert payload["result"]["kind"] == MESSAGE_LABEL
    assert payload["result"]["role"] == AGENT_ROLE
    assert payload["result"]["parts"][0]["text"] == REPLY_TEXT
    assert {"id", "jsonrpc", "result", "error", "usage", "_hidden_params"} <= set(unstructured)
    assert unstructured["_hidden_params"] == {}


def test_send_message_carries_the_client_headers() -> None:
    recorder = Recorder()
    extra_headers: dict[str, JsonValue] = {EXTRA_HEADER: EXTRA_VALUE}
    with AgentServer(recorder) as server:
        settings = settings_of(server, extra_headers=extra_headers)
        asyncio.run(send_with(settings, send_call()))
    gets = recorded(recorder, "GET")
    posts = recorded(recorder, "POST")
    assert len(gets) == SINGLE
    assert len(posts) == SINGLE
    assert posts[0]["path"] == "/"
    assert header_of(gets[0], EXTRA_HEADER) == EXTRA_VALUE
    assert header_of(posts[0], EXTRA_HEADER) == EXTRA_VALUE


def test_send_message_direct_sets_the_agent_headers() -> None:
    recorder = Recorder()
    agent_headers: dict[str, JsonValue] = {TRACE_HEADER: AGENT_TRACE}
    with AgentServer(recorder) as server:
        call = send_call(
            api_base=server.url,
            trace_id=TRACE_ID,
            agent_id=AGENT_ID,
            agent_extra_headers=agent_headers,
        )
        text = asyncio.run(direct_reply(call))
    payload = TypeAdapter(SendView).validate_json(text)
    posts = recorded(recorder, "POST")
    assert payload["result"]["parts"][0]["text"] == REPLY_TEXT
    assert len(posts) == SINGLE
    assert header_of(posts[0], TRACE_HEADER) == AGENT_TRACE
    assert header_of(posts[0], AGENT_HEADER) == AGENT_ID
    assert call_of(posts[0]).get("method") == SEND_METHOD


def test_send_message_streaming_gives_the_events_in_order() -> None:
    recorder = Recorder()
    with AgentServer(recorder) as server:
        settings = settings_of(server, streaming=True)
        chunks = asyncio.run(stream_with(settings, send_call(request_id=STREAM_REQUEST_ID)))
    assert len(chunks) == STREAM_EVENTS
    status = TypeAdapter(StatusChunk).validate_json(chunks[0])
    assert status["result"]["kind"] == STATUS_LABEL
    assert status["result"]["taskId"] == TASK_ID
    assert status["result"]["status"]["state"] == WORKING_STATE
    assert status["result"]["final"] is False
    artifact = TypeAdapter(ArtifactChunk).validate_json(chunks[1])
    assert artifact["result"]["kind"] == ARTIFACT_LABEL
    assert artifact["result"]["artifact"]["parts"][0]["text"] == ARTIFACT_TEXT
    message = TypeAdapter(MessageChunk).validate_json(chunks[2])
    assert message["result"]["kind"] == MESSAGE_LABEL
    assert message["result"]["parts"][0]["text"] == REPLY_TEXT
    posts = recorded(recorder, "POST")
    assert call_of(posts[0]).get("method") == STREAM_METHOD
    assert header_of(posts[0], ACCEPT_HEADER) == EVENT_STREAM


def test_send_message_streaming_closes_midstream() -> None:
    with AgentServer(Recorder()) as server:
        settings = settings_of(server, streaming=True)
        first = asyncio.run(first_then_close(settings, send_call()))
    status = TypeAdapter(StatusChunk).validate_json(first)
    assert status["result"]["kind"] == STATUS_LABEL


def test_send_message_streaming_direct_sets_the_trace_header() -> None:
    recorder = Recorder()
    with AgentServer(recorder) as server:
        call = send_call(api_base=server.url, request_id=STREAM_REQUEST_ID)
        chunks = asyncio.run(stream_direct(call))
    assert len(chunks) == STREAM_EVENTS
    posts = recorded(recorder, "POST")
    assert header_of(posts[0], TRACE_HEADER) == STREAM_REQUEST_ID


def test_get_agent_card_returns_the_card() -> None:
    recorder = Recorder()
    extra_headers: dict[str, JsonValue] = {EXTRA_HEADER: EXTRA_VALUE}
    with AgentServer(recorder) as server:
        settings = settings_of(server, extra_headers=extra_headers)
        text = asyncio.run(card_for(settings))
    card = TypeAdapter(CardView).validate_json(text)
    unstructured = TypeAdapter(dict[str, JsonValue]).validate_json(text)
    assert card["name"] == AGENT_NAME
    assert card["protocolVersion"] == PROTOCOL_VERSION
    assert card["url"] == server.base_url
    assert card["capabilities"]["streaming"] is True
    assert "supportedInterfaces" not in unstructured
    gets = recorded(recorder, "GET")
    assert [call["path"] for call in gets] == [CARD_PATH]
    assert header_of(gets[0], EXTRA_HEADER) == EXTRA_VALUE


def test_get_agent_card_probes_the_legacy_path() -> None:
    recorder = Recorder(legacy_card=True)
    with AgentServer(recorder) as server:
        text = asyncio.run(card_for(settings_of(server)))
    assert TypeAdapter(CardView).validate_json(text)["name"] == AGENT_NAME
    gets = recorded(recorder, "GET")
    assert [call["path"] for call in gets] == [CARD_PATH, LEGACY_CARD_PATH]
    assert len(gets) == CARD_PROBES


def test_get_agent_card_uses_the_relative_card_path() -> None:
    recorder = Recorder(legacy_card=True)
    with AgentServer(recorder) as server:
        settings = settings_of(server, relative_card_path=LEGACY_CARD_PATH)
        text = asyncio.run(card_for(settings))
    assert TypeAdapter(CardView).validate_json(text)["name"] == AGENT_NAME
    gets = recorded(recorder, "GET")
    assert [call["path"] for call in gets] == [LEGACY_CARD_PATH]


def test_create_client_returns_the_normalized_card() -> None:
    with AgentServer(Recorder(interfaces=True)) as server:
        settings = settings_of(server)
        base_url, text = asyncio.run(created_card(settings))
    card = TypeAdapter(InterfaceCard).validate_json(text)
    interface = card["supportedInterfaces"][0]
    assert base_url == server.url
    assert interface["url"] == server.base_url
    assert interface["protocolBinding"] == "JSONRPC"
    assert interface["protocolVersion"] == "0.3"


def test_card_helpers_match_the_library() -> None:
    local: dict[str, JsonValue] = {
        "protocolVersion": PROTOCOL_VERSION,
        "name": AGENT_NAME,
        "url": LOCALHOST_URL,
    }
    textual = json.dumps(local)
    assert _native.is_localhost_or_internal_url(LOCALHOST_URL)
    assert _native.is_localhost_or_internal_url("http://127.0.0.1:9/")
    assert not _native.is_localhost_or_internal_url("https://agent.example")
    assert not _native.is_localhost_or_internal_url(None)
    assert _native.get_agent_card_url(textual) == LOCALHOST_URL
    fixed_text = _native.fix_agent_card_url(textual, "https://agent.example")
    assert TypeAdapter(UrlCard).validate_json(fixed_text)["url"] == "https://agent.example/"
    moved_text = _native.set_agent_card_url(textual, "https://other.example/")
    assert TypeAdapter(UrlCard).validate_json(moved_text)["url"] == "https://other.example/"
    assert _native.get_agent_card_url(moved_text) == "https://other.example/"
    assert _native.get_agent_card_url(json.dumps({"name": AGENT_NAME})) is None
    interfaces: dict[str, JsonValue] = {
        "name": AGENT_NAME,
        "supportedInterfaces": [
            {
                "url": "https://first.example/",
                "protocolBinding": "jsonrpc",
                "protocolVersion": "1.0",
            },
            {
                "url": "https://second.example/",
                "protocolBinding": "JSONRPC",
                "protocolVersion": "1.0",
            },
        ],
    }
    assert _native.get_agent_card_url(json.dumps(interfaces)) == "https://first.example/"
    normalized_text = _native.normalize_agent_card_interfaces(json.dumps(interfaces))
    card = TypeAdapter(InterfaceCard).validate_json(normalized_text)
    normalized = card["supportedInterfaces"][0]
    assert normalized["protocolBinding"] == "JSONRPC"
    assert normalized["protocolVersion"] == "0.3"


def test_errors_report_a_connection_failure() -> None:
    with AgentServer(Recorder(card_url=DEAD_URL)) as server:
        settings = settings_of(server)
        with pytest.raises(_native.NativeA2aError) as raised:
            asyncio.run(failing_send(settings, send_call()))
    payload = TypeAdapter(ErrorView).validate_json(str(raised.value))
    assert payload["type"] == "A2AConnectionError"
    assert payload["status_code"] == DEAD_STATUS
    assert payload["llm_provider"] == LLM_PROVIDER
    assert payload["url"] == DEAD_URL
    assert payload["message"]


def test_errors_state_a_missing_request() -> None:
    call: dict[str, JsonValue] = {"api_base": "http://127.0.0.1:1"}
    with pytest.raises(_native.NativeA2aError) as raised:
        asyncio.run(failing_direct(json.dumps(call)))
    payload = TypeAdapter(ErrorView).validate_json(str(raised.value))
    assert payload["type"] == "ValueError"
    assert payload["message"] == "request is required"
    assert payload["status_code"] is None


def test_errors_state_a_missing_client() -> None:
    call: dict[str, JsonValue] = {"request": request_of(REQUEST_ID)}
    with pytest.raises(_native.NativeA2aError) as raised:
        asyncio.run(failing_direct(json.dumps(call)))
    payload = TypeAdapter(ErrorView).validate_json(str(raised.value))
    assert payload["type"] == "ValueError"
    assert payload["message"] == MISSING_CLIENT


def test_errors_reject_malformed_call_text() -> None:
    with pytest.raises(_native.NativeA2aError) as raised:
        _ = _native.send_message_direct("{")
    payload = TypeAdapter(ErrorView).validate_json(str(raised.value))
    assert payload["type"] == "ValueError"
    assert "not JSON of the expected shape" in payload["message"]


def test_errors_recover_from_a_localhost_card() -> None:
    recorder = Recorder(card_url=LOCALHOST_URL)
    with AgentServer(recorder) as server:
        call = send_call(api_base=server.url)
        text = asyncio.run(direct_reply(call))
    payload = TypeAdapter(SendView).validate_json(text)
    posts = recorded(recorder, "POST")
    assert payload["result"]["parts"][0]["text"] == REPLY_TEXT
    assert len(recorded(recorder, "GET")) == SINGLE
    assert len(posts) == SINGLE
    assert call_of(posts[0]).get("method") == SEND_METHOD
