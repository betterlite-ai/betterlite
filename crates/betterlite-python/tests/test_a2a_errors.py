import asyncio
import json

import pytest
from a2a_server import (
    CARD_PATH,
    DEAD_URL,
    LEGACY_CARD_PATH,
    LOCALHOST_URL,
    AgentServer,
    Recorder,
)
from litellm.a2a_protocol import exceptions as their_errors
from litellm.a2a_protocol import main as litellm_main
from pydantic import JsonValue

from betterlite._native import NativeA2aError
from betterlite.a2a_protocol import exceptions, main
from betterlite.a2a_protocol.errors import (
    A2A_ERROR_TYPES,
    UnknownNativeError,
    UnsupportedA2AArgument,
    raise_native_error,
)
from betterlite.a2a_protocol.types import SendMessageRequest

REQUEST_ID = "req-1"
USER_MESSAGE_ID = "user-message"
USER_TEXT = "hi"
MODEL = "agent"
LLM_PROVIDER = "a2a_agent"
AGENT_URL = "https://agent.example"
API_BASE = "http://127.0.0.1:1"
MAX_RETRIES = 2
NUM_RETRIES = 1
BASE_STATUS = 500
TEAPOT = 418
DEAD_STATUS = 503
CARD_STATUS = 404
CARD_PROBES = 3
DISCOVERY_FAILURES = 2


def payload_of(kind: str, message: str) -> dict[str, JsonValue]:
    payload: dict[str, JsonValue] = {
        "type": kind,
        "message": message,
        "status_code": None,
        "llm_provider": None,
        "url": None,
        "model": None,
        "litellm_debug_info": None,
        "max_retries": None,
        "num_retries": None,
        "base_url": None,
        "localhost_url": None,
        "failures": None,
        "original_error": None,
    }
    return payload


def native_of(**fields: JsonValue) -> NativeA2aError:
    text = json.dumps(fields)
    return NativeA2aError(text)


def request_params() -> dict[str, JsonValue]:
    message: dict[str, JsonValue] = {
        "role": "user",
        "parts": [{"kind": "text", "text": USER_TEXT}],
        "messageId": USER_MESSAGE_ID,
    }
    return {"message": message}


def message_request() -> SendMessageRequest:
    params = request_params()
    return SendMessageRequest(id=REQUEST_ID, params=params)


async def failing_send(api_base: str) -> None:
    request = message_request()
    await main.asend_message(api_base=api_base, request=request)


def test_errors_match_litellm() -> None:
    ours = exceptions.A2AError(
        "boom", model=MODEL, max_retries=MAX_RETRIES, num_retries=NUM_RETRIES
    )
    theirs = their_errors.A2AError(
        "boom", model=MODEL, max_retries=MAX_RETRIES, num_retries=NUM_RETRIES
    )
    assert ours.message == theirs.message
    assert ours.status_code == theirs.status_code == BASE_STATUS
    assert ours.llm_provider == theirs.llm_provider
    assert ours.model == theirs.model
    assert ours.litellm_debug_info == theirs.litellm_debug_info
    assert ours.max_retries == theirs.max_retries
    assert ours.num_retries == theirs.num_retries
    assert str(ours) == str(theirs)
    assert repr(ours) == repr(theirs)
    assert ours.response.status_code == theirs.response.status_code
    assert str(ours.response.request.url) == str(theirs.response.request.url)
    assert ours.response.request.method == theirs.response.request.method
    plain = exceptions.A2AError("boom")
    assert plain.message == "litellm.A2AError: boom"


def test_connection_error_match() -> None:
    ours = exceptions.A2AConnectionError("down", url=DEAD_URL, model=MODEL)
    theirs = their_errors.A2AConnectionError("down", url=DEAD_URL, model=MODEL)
    assert ours.status_code == theirs.status_code == DEAD_STATUS
    assert ours.llm_provider == theirs.llm_provider == LLM_PROVIDER
    assert ours.url == theirs.url
    assert ours.message == theirs.message
    assert str(ours) == str(theirs)


def test_card_error_match() -> None:
    ours = exceptions.A2AAgentCardError("bad card", url=AGENT_URL)
    theirs = their_errors.A2AAgentCardError("bad card", url=AGENT_URL)
    assert ours.status_code == theirs.status_code == CARD_STATUS
    assert ours.url == theirs.url
    assert ours.llm_provider == theirs.llm_provider == LLM_PROVIDER
    assert str(ours) == str(theirs)


def test_discovery_error_match() -> None:
    failures: tuple[tuple[str, Exception], ...] = (
        (CARD_PATH, RuntimeError("missing")),
        (LEGACY_CARD_PATH, RuntimeError("missing")),
    )
    ours = exceptions.A2AAgentCardDiscoveryError(AGENT_URL, failures, CARD_STATUS)
    theirs = their_errors.A2AAgentCardDiscoveryError(AGENT_URL, failures, CARD_STATUS)
    assert str(ours) == str(theirs)
    assert ours.failures == theirs.failures
    assert ours.url == theirs.url == AGENT_URL
    assert ours.status_code == theirs.status_code == CARD_STATUS


def test_localhost_error_match() -> None:
    original = RuntimeError("refused")
    ours = exceptions.A2ALocalhostURLError(LOCALHOST_URL, AGENT_URL, original, model=MODEL)
    theirs = their_errors.A2ALocalhostURLError(LOCALHOST_URL, AGENT_URL, original, model=MODEL)
    assert str(ours) == str(theirs)
    assert ours.localhost_url == theirs.localhost_url
    assert ours.base_url == theirs.base_url
    assert ours.url == theirs.url == LOCALHOST_URL
    assert ours.status_code == theirs.status_code == DEAD_STATUS
    assert ours.original_error is theirs.original_error


def test_native_a2a_error() -> None:
    payload = payload_of("A2AError", "boom")
    payload["status_code"] = TEAPOT
    payload["llm_provider"] = LLM_PROVIDER
    payload["model"] = MODEL
    payload["litellm_debug_info"] = "debug"
    payload["max_retries"] = MAX_RETRIES
    payload["num_retries"] = NUM_RETRIES
    with pytest.raises(exceptions.A2AError) as caught:
        raise_native_error(native_of(**payload))
    error = caught.value
    assert error.status_code == TEAPOT
    assert error.response.status_code == TEAPOT
    assert error.llm_provider == LLM_PROVIDER
    assert error.model == MODEL
    assert error.litellm_debug_info == "debug"
    assert error.max_retries == MAX_RETRIES
    assert error.num_retries == NUM_RETRIES
    assert str(error).startswith("litellm.A2AError: boom")
    assert error.__cause__ is not None


def test_native_status_errors() -> None:
    connection = payload_of("A2AConnectionError", "down")
    connection["status_code"] = TEAPOT
    connection["url"] = DEAD_URL
    with pytest.raises(exceptions.A2AConnectionError) as caught_connection:
        raise_native_error(native_of(**connection))
    connection_error = caught_connection.value
    assert connection_error.status_code == DEAD_STATUS
    assert connection_error.llm_provider == LLM_PROVIDER
    assert connection_error.url == DEAD_URL
    assert connection_error.__cause__ is not None
    card = payload_of("A2AAgentCardError", "bad card")
    card["status_code"] = TEAPOT
    card["url"] = AGENT_URL
    with pytest.raises(exceptions.A2AAgentCardError) as caught_card:
        raise_native_error(native_of(**card))
    card_error = caught_card.value
    assert card_error.status_code == TEAPOT
    assert card_error.url == AGENT_URL
    assert card_error.llm_provider == LLM_PROVIDER


def test_native_discovery_error() -> None:
    failures: list[JsonValue] = [
        {"path": CARD_PATH, "error": "missing"},
        {"path": LEGACY_CARD_PATH, "error": "missing"},
    ]
    payload = payload_of("A2AAgentCardDiscoveryError", "no card")
    payload["status_code"] = CARD_STATUS
    payload["base_url"] = AGENT_URL
    payload["failures"] = failures
    with pytest.raises(exceptions.A2AAgentCardDiscoveryError) as caught:
        raise_native_error(native_of(**payload))
    error = caught.value
    assert error.status_code == CARD_STATUS
    assert error.url == AGENT_URL
    assert len(error.failures) == DISCOVERY_FAILURES
    assert error.failures[0][0] == CARD_PATH
    assert str(error.failures[0][1]) == "missing"
    theirs = their_errors.A2AAgentCardDiscoveryError(AGENT_URL, error.failures, CARD_STATUS)
    assert str(error) == str(theirs)


def test_native_localhost_error() -> None:
    payload = payload_of("A2ALocalhostURLError", "ignored")
    payload["localhost_url"] = LOCALHOST_URL
    payload["base_url"] = AGENT_URL
    payload["model"] = MODEL
    payload["original_error"] = "refused"
    with pytest.raises(exceptions.A2ALocalhostURLError) as caught:
        raise_native_error(native_of(**payload))
    error = caught.value
    assert error.localhost_url == LOCALHOST_URL
    assert error.base_url == AGENT_URL
    assert error.url == LOCALHOST_URL
    assert error.status_code == DEAD_STATUS
    assert error.model == MODEL
    assert error.original_error is not None
    assert str(error.original_error) == "refused"
    theirs = their_errors.A2ALocalhostURLError(
        LOCALHOST_URL, AGENT_URL, RuntimeError("refused"), model=MODEL
    )
    assert str(error) == str(theirs)


def test_native_value_error() -> None:
    payload = payload_of("ValueError", "request is required")
    with pytest.raises(ValueError, match="request is required") as caught:
        raise_native_error(native_of(**payload))
    assert caught.value.__cause__ is not None


def test_native_runtime_error() -> None:
    payload = payload_of("RuntimeError", "the answer could not be written")
    with pytest.raises(RuntimeError, match="the answer could not be written") as caught:
        raise_native_error(native_of(**payload))
    assert caught.value.__cause__ is not None


def test_native_unknown_error() -> None:
    payload = payload_of("TimeoutError", "slow")
    native = native_of(**payload)
    with pytest.raises(UnknownNativeError) as caught:
        raise_native_error(native)
    assert caught.value.payload == str(native)
    assert caught.value.__cause__ is native


def test_error_table() -> None:
    assert A2A_ERROR_TYPES["A2AError"] is exceptions.A2AError
    assert A2A_ERROR_TYPES["A2AConnectionError"] is exceptions.A2AConnectionError
    assert A2A_ERROR_TYPES["A2AAgentCardError"] is exceptions.A2AAgentCardError
    assert A2A_ERROR_TYPES["A2AAgentCardDiscoveryError"] is exceptions.A2AAgentCardDiscoveryError
    assert A2A_ERROR_TYPES["A2ALocalhostURLError"] is exceptions.A2ALocalhostURLError
    assert A2A_ERROR_TYPES["ValueError"] is ValueError
    assert A2A_ERROR_TYPES["RuntimeError"] is RuntimeError


def test_unsupported_argument() -> None:
    error = UnsupportedA2AArgument("a2a_client")
    assert error.argument == "a2a_client"
    assert "a2a_client" in str(error)


def test_missing_request() -> None:
    with pytest.raises(ValueError, match="request is required") as caught:
        asyncio.run(main.asend_message(api_base=API_BASE))
    assert caught.value.__cause__ is not None


def test_missing_client() -> None:
    request = message_request()
    with pytest.raises(ValueError, match="Either a2a_client or api_base is required") as caught:
        asyncio.run(main.asend_message(request=request))
    assert caught.value.__cause__ is not None


def test_connection_at_agent() -> None:
    with (
        AgentServer(Recorder(card_url=DEAD_URL)) as server,
        pytest.raises(exceptions.A2AConnectionError) as caught,
    ):
        asyncio.run(failing_send(server.url))
    error = caught.value
    assert error.status_code == DEAD_STATUS
    assert error.llm_provider == LLM_PROVIDER
    assert error.url == DEAD_URL
    assert str(error).startswith("litellm.A2AError: ")


def test_discovery_at_agent() -> None:
    with AgentServer(Recorder()) as server:
        base = f"{server.url}/missing"
        with pytest.raises(exceptions.A2AAgentCardDiscoveryError) as caught:
            asyncio.run(main.aget_agent_card(base))
        with pytest.raises(their_errors.A2AAgentCardDiscoveryError) as their_caught:
            asyncio.run(litellm_main.aget_agent_card(base))
    ours = caught.value
    theirs = their_caught.value
    prefix = f"litellm.A2AError: Failed to fetch agent card from {base}. Tried "
    assert str(ours).startswith(prefix)
    assert str(theirs).startswith(prefix)
    assert ours.status_code == theirs.status_code == CARD_STATUS
    assert ours.url == theirs.url == base
    assert len(ours.failures) == len(theirs.failures) == CARD_PROBES
    assert ours.failures[0][0] == theirs.failures[0][0] == CARD_PATH
