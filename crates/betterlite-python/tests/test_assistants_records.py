import asyncio
import importlib
import inspect
from collections.abc import Callable, Coroutine, Sequence
from typing import Final, Protocol, TypeAlias
from urllib.parse import parse_qsl, urlsplit

import litellm.exceptions as their_errors
import openai
import pytest
from assistant_server import (
    ASSISTANT_ID,
    BAD_KEY,
    GOOD_KEY,
    THREAD_ID,
    AssistantServer,
    RecordedCall,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import runtime_checkable

import betterlite.exceptions as errors
from betterlite._completion.errors import UnsupportedCallArgumentError
from betterlite._endpoints.payload import CallValue
from betterlite.assistants import _records
from betterlite.types.utils import Message

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
UNAUTHORIZED_STATUS: Final = 401
BAD_REQUEST_STATUS: Final = 400
FAKE_CLIENT: Final = "openai.OpenAI"
COMPARED_HEADERS: Final[tuple[str, ...]] = ("authorization", "openai-beta")
CREATE: Final[dict[str, CallValue]] = {
    "model": "gpt-4o-mini",
    "name": "helper",
    "description": "a helper",
    "instructions": "be brief",
    "tools": [{"type": "code_interpreter"}],
    "tool_resources": {"code_interpreter": {"file_ids": ["file_1"]}},
    "metadata": {"team": "a"},
    "temperature": 0.0,
    "top_p": 0.0,
    "response_format": "auto",
}
THREAD: Final[dict[str, CallValue]] = {
    "messages": [{"role": "user", "content": "hi"}],
    "metadata": {"team": "a"},
    "tool_resources": {"code_interpreter": {"file_ids": ["file_1"]}},
}


@runtime_checkable
class Dumpable(Protocol):
    def model_dump_json(self) -> str: ...


@runtime_checkable
class Page(Protocol):
    data: Sequence[Dumpable]
    has_more: bool


Answer: TypeAlias = Dumpable | Page | Coroutine[None, None, Dumpable | Page | None] | None
SyncCall: TypeAlias = Callable[..., Answer]


class Reference(Protocol):
    def __getattr__(self, name: str) -> SyncCall: ...


LITELLM: Reference = importlib.import_module("litellm")
FUNCTIONS: Final[dict[str, Callable[..., Answer]]] = {
    "get_assistants": _records.get_assistants,
    "create_assistants": _records.create_assistants,
    "delete_assistant": _records.delete_assistant,
    "create_thread": _records.create_thread,
    "get_thread": _records.get_thread,
    "aget_assistants": _records.aget_assistants,
    "acreate_assistants": _records.acreate_assistants,
    "adelete_assistant": _records.adelete_assistant,
    "acreate_thread": _records.acreate_thread,
    "aget_thread": _records.aget_thread,
}
REFERENCE_FUNCTIONS: Final[dict[str, SyncCall]] = {
    "get_assistants": LITELLM.get_assistants,
    "create_assistants": LITELLM.create_assistants,
    "delete_assistant": LITELLM.delete_assistant,
    "create_thread": LITELLM.create_thread,
    "get_thread": LITELLM.get_thread,
    "aget_assistants": LITELLM.aget_assistants,
    "acreate_assistants": LITELLM.acreate_assistants,
    "adelete_assistant": LITELLM.adelete_assistant,
    "acreate_thread": LITELLM.acreate_thread,
    "aget_thread": LITELLM.aget_thread,
}


def shape(function: Callable[..., Answer]) -> list[tuple[str, str, bool]]:
    parameters = inspect.signature(function).parameters
    return [
        (name, str(parameter.kind), parameter.default is None)
        for name, parameter in parameters.items()
    ]


def dump_of(model: Dumpable) -> JsonValue:
    dumped = model.model_dump_json()
    return JSON.validate_json(dumped)


def answer_of(answer: Answer) -> Dumpable:
    match answer:
        case Dumpable() as model:
            return model
        case Coroutine():
            raise AssertionError("the call answered a coroutine without its flag")
        case object():
            raise AssertionError("the call answered no object")


def page_of(answer: Answer) -> Page:
    match answer:
        case Page() as page:
            return page
        case Coroutine():
            raise AssertionError("the call answered a coroutine without its flag")
        case object():
            raise AssertionError("the call answered no page")


def page_view(page: Page) -> dict[str, JsonValue]:
    data: list[JsonValue] = [dump_of(item) for item in page.data]
    return {"data": data, "has_more": page.has_more}


def called_path(call: RecordedCall) -> dict[str, JsonValue]:
    parts = urlsplit(call["path"])
    return {"path": parts.path, "query": dict(parse_qsl(parts.query))}


def called_headers(call: RecordedCall) -> dict[str, str]:
    return {
        name.lower(): value
        for name, value in call["headers"].items()
        if name.lower() in COMPARED_HEADERS
    }


def same_call(reference: RecordedCall, ours: RecordedCall) -> None:
    assert ours["method"] == reference["method"]
    assert called_path(ours) == called_path(reference)
    assert ours.get("body") == reference.get("body")
    assert called_headers(ours) == called_headers(reference)


def test_signature_matches_litellm() -> None:
    for name, ours in FUNCTIONS.items():
        assert shape(ours) == shape(REFERENCE_FUNCTIONS[name])


def test_calls_list_assistants() -> None:
    with AssistantServer() as server:
        reference = page_of(
            LITELLM.get_assistants("openai", api_key=GOOD_KEY, api_base=server.url)
        )
        reference_call = server.calls.pop()
        ours = page_of(_records.get_assistants("openai", api_key=GOOD_KEY, api_base=server.url))
        same_call(reference_call, server.calls.pop())
        assert page_view(ours) == page_view(reference)


def test_calls_create_assistant() -> None:
    with AssistantServer() as server:
        reference = answer_of(
            LITELLM.create_assistants("openai", **CREATE, api_key=GOOD_KEY, api_base=server.url)
        )
        reference_call = server.calls.pop()
        ours = answer_of(
            _records.create_assistants("openai", **CREATE, api_key=GOOD_KEY, api_base=server.url)
        )
        same_call(reference_call, server.calls.pop())
        assert dump_of(ours) == dump_of(reference)


def test_calls_delete_assistant() -> None:
    with AssistantServer() as server:
        reference = answer_of(
            LITELLM.delete_assistant(
                "openai", ASSISTANT_ID, api_key=GOOD_KEY, api_base=server.url
            )
        )
        reference_call = server.calls.pop()
        ours = answer_of(
            _records.delete_assistant(
                "openai", ASSISTANT_ID, api_key=GOOD_KEY, api_base=server.url
            )
        )
        same_call(reference_call, server.calls.pop())
        assert dump_of(ours) == dump_of(reference)


def test_calls_threads() -> None:
    with AssistantServer() as server:
        reference = answer_of(
            LITELLM.create_thread("openai", **THREAD, api_key=GOOD_KEY, api_base=server.url)
        )
        reference_call = server.calls.pop()
        ours = answer_of(
            _records.create_thread("openai", **THREAD, api_key=GOOD_KEY, api_base=server.url)
        )
        same_call(reference_call, server.calls.pop())
        assert dump_of(ours) == dump_of(reference)
        reference_got = answer_of(
            LITELLM.get_thread("openai", THREAD_ID, api_key=GOOD_KEY, api_base=server.url)
        )
        reference_thread_call = server.calls.pop()
        ours_got = answer_of(
            _records.get_thread("openai", THREAD_ID, api_key=GOOD_KEY, api_base=server.url)
        )
        same_call(reference_thread_call, server.calls.pop())
        assert dump_of(ours_got) == dump_of(reference_got)


def test_calls_message_object() -> None:
    dumped = _records.thread_message(Message(content="hi", role="user"))
    assert dumped == {"role": "user", "content": "hi"}


def test_async_flag_coroutines() -> None:
    with AssistantServer() as server:
        flagged = (
            _records.get_assistants(
                "openai", api_key=GOOD_KEY, api_base=server.url, aget_assistants=True
            ),
            _records.create_assistants(
                "openai", "gpt-4o-mini", api_key=GOOD_KEY, api_base=server.url,
                async_create_assistants=True,
            ),
            _records.delete_assistant(
                "openai", ASSISTANT_ID, api_key=GOOD_KEY, api_base=server.url,
                async_delete_assistants=True,
            ),
            _records.create_thread(
                "openai", messages=[{"role": "user", "content": "hi"}], api_key=GOOD_KEY,
                api_base=server.url, acreate_thread=True,
            ),
            _records.get_thread(
                "openai", THREAD_ID, api_key=GOOD_KEY, api_base=server.url, aget_thread=True
            ),
        )
        for answer in flagged:
            match answer:
                case Coroutine():
                    answer.close()
                case object():
                    raise AssertionError("the flag did not answer a coroutine")
        page = asyncio.run(
            _records.aget_assistants("openai", api_key=GOOD_KEY, api_base=server.url)
        )
        assert page.has_more is False
        thread = asyncio.run(
            _records.acreate_thread(
                "openai", messages=[{"role": "user", "content": "hi"}],
                api_key=GOOD_KEY, api_base=server.url,
            )
        )
        assert thread is not None
        assert thread.id == THREAD_ID
        number_flag = _records.create_thread(
            "openai", messages=[{"role": "user", "content": "hi"}], api_key=GOOD_KEY,
            api_base=server.url, acreate_thread=1,
        )
        assert not inspect.iscoroutine(number_flag)


def test_error_class_and_client() -> None:
    with AssistantServer() as server:
        with pytest.raises(errors.AuthenticationError) as ours:
            _records.get_assistants("openai", api_key=BAD_KEY, api_base=server.url)
        with pytest.raises(openai.AuthenticationError) as reference:
            LITELLM.get_assistants("openai", api_key=BAD_KEY, api_base=server.url)
        assert type(ours.value).__name__ == type(reference.value).__name__
        assert ours.value.status_code == UNAUTHORIZED_STATUS
        assert ours.value.model == ""
        assert "Incorrect API key" in str(ours.value)
        assert "Incorrect API key" in str(reference.value)
        with pytest.raises(errors.BadRequestError) as ours_unsupported:
            _records.get_assistants("bedrock", api_key=GOOD_KEY, api_base=server.url)
        with pytest.raises(their_errors.BadRequestError) as reference_unsupported:
            LITELLM.get_assistants("bedrock", api_key=GOOD_KEY, api_base=server.url)
        assert str(ours_unsupported.value) == str(reference_unsupported.value)
        assert ours_unsupported.value.model == "n/a"
        assert ours_unsupported.value.status_code == BAD_REQUEST_STATUS
        with pytest.raises(UnsupportedCallArgumentError, match="client"):
            _records.get_assistants(
                "openai", client=FAKE_CLIENT, api_key=GOOD_KEY, api_base=server.url
            )
        with pytest.raises(ValueError, match="async_create_assistants"):
            _records.create_assistants("openai", "gpt-4o-mini", async_create_assistants=1)
        with pytest.raises(ValueError, match="async_delete_assistants"):
            _records.delete_assistant("openai", ASSISTANT_ID, async_delete_assistants="yes")
        with pytest.raises(Exception, match="aget_assistants"):
            _records.get_assistants("openai", aget_assistants=1)
