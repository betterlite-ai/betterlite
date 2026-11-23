import asyncio
import importlib
import inspect
from collections.abc import Coroutine, Sequence
from typing import Final, Protocol, TypeAlias
from urllib.parse import parse_qsl, urlsplit

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
RefResult: TypeAlias = Dumpable | Page | None
CallArgument: TypeAlias = JsonValue


class SyncCall(Protocol):
    def __call__(self, *args: CallArgument, **kwargs: CallArgument) -> RefResult: ...


class LiteLLMReference(Protocol):
    def __getattr__(self, name: str) -> SyncCall: ...


class LiteLLMErrors(Protocol):
    def __getattr__(self, name: str) -> type[Exception]: ...


LITELLM: LiteLLMReference = importlib.import_module("litellm")
THEIR_ERRORS: LiteLLMErrors = importlib.import_module("litellm.exceptions")


def shape(signature: inspect.Signature) -> list[tuple[str, str, bool]]:
    parameters = signature.parameters
    return [
        (name, parameter.kind.name, parameter.default is None)
        for name, parameter in parameters.items()
    ]


def dump_pair(ours: Dumpable, reference: Dumpable) -> None:
    ours_json = JSON.validate_json(ours.model_dump_json())
    reference_json = JSON.validate_json(reference.model_dump_json())
    assert ours_json == reference_json


def answer_of(answer: Answer) -> Dumpable:
    match answer:
        case Dumpable() as model:
            return model
        case object():
            raise AssertionError("the call answered no object")


def page_of(answer: Answer) -> Page:
    match answer:
        case Page() as page:
            return page
        case object():
            raise AssertionError("the call answered no page")


def page_view(page: Page) -> dict[str, JsonValue]:
    data: list[JsonValue] = [JSON.validate_json(item.model_dump_json()) for item in page.data]
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


def matched_calls(server: AssistantServer) -> None:
    ours = server.calls.pop()
    same_call(server.calls.pop(), ours)


def create_reference(server: AssistantServer) -> RefResult:
    arguments: dict[str, CallArgument] = {**CREATE, "api_key": GOOD_KEY, "api_base": server.url}
    return LITELLM.create_assistants("openai", **arguments)


def create_ours(server: AssistantServer) -> Answer:
    return _records.create_assistants(
        "openai", model="gpt-4o-mini", name="helper", description="a helper",
        instructions="be brief", tools=[{"type": "code_interpreter"}],
        tool_resources={"code_interpreter": {"file_ids": ["file_1"]}},
        metadata={"team": "a"}, temperature=0.0, top_p=0.0, response_format="auto",
        api_key=GOOD_KEY, api_base=server.url,
    )


def test_signature_matches_litellm() -> None:
    assert shape(inspect.signature(_records.get_assistants)) == shape(
        inspect.signature(LITELLM.get_assistants))
    assert shape(inspect.signature(_records.aget_assistants)) == shape(
        inspect.signature(LITELLM.aget_assistants))
    assert shape(inspect.signature(_records.create_assistants)) == shape(
        inspect.signature(LITELLM.create_assistants))
    assert shape(inspect.signature(_records.acreate_assistants)) == shape(
        inspect.signature(LITELLM.acreate_assistants))
    assert shape(inspect.signature(_records.delete_assistant)) == shape(
        inspect.signature(LITELLM.delete_assistant))
    assert shape(inspect.signature(_records.adelete_assistant)) == shape(
        inspect.signature(LITELLM.adelete_assistant))
    assert shape(inspect.signature(_records.create_thread)) == shape(
        inspect.signature(LITELLM.create_thread))
    assert shape(inspect.signature(_records.acreate_thread)) == shape(
        inspect.signature(LITELLM.acreate_thread))
    assert shape(inspect.signature(_records.get_thread)) == shape(
        inspect.signature(LITELLM.get_thread))
    assert shape(inspect.signature(_records.aget_thread)) == shape(
        inspect.signature(LITELLM.aget_thread))


def test_calls_list_assistants() -> None:
    with AssistantServer() as server:
        args = {"api_key": GOOD_KEY, "api_base": server.url}
        reference = page_of(LITELLM.get_assistants("openai", **args))
        ours = page_of(_records.get_assistants("openai", **args))
        matched_calls(server)
        assert page_view(ours) == page_view(reference)


def test_calls_create_assistant() -> None:
    with AssistantServer() as server:
        reference = answer_of(create_reference(server))
        ours = answer_of(create_ours(server))
        matched_calls(server)
        dump_pair(ours, reference)


def test_calls_delete_assistant() -> None:
    with AssistantServer() as server:
        args = {"api_key": GOOD_KEY, "api_base": server.url}
        reference = answer_of(LITELLM.delete_assistant("openai", ASSISTANT_ID, **args))
        ours = answer_of(_records.delete_assistant("openai", ASSISTANT_ID, **args))
        matched_calls(server)
        dump_pair(ours, reference)


def test_calls_threads() -> None:
    with AssistantServer() as server:
        args = {"api_key": GOOD_KEY, "api_base": server.url}
        reference = answer_of(LITELLM.create_thread("openai", **THREAD, **args))
        ours = answer_of(_records.create_thread("openai", **THREAD, **args))
        matched_calls(server)
        dump_pair(ours, reference)
        reference_got = answer_of(LITELLM.get_thread("openai", THREAD_ID, **args))
        ours_got = answer_of(_records.get_thread("openai", THREAD_ID, **args))
        matched_calls(server)
        dump_pair(ours_got, reference_got)


def test_calls_message_object() -> None:
    dumped = _records.thread_message(Message(content="hi", role="user"))
    assert dumped == {"role": "user", "content": "hi"}


def test_async_flag_coroutines() -> None:
    with AssistantServer() as server:
        args = {"api_key": GOOD_KEY, "api_base": server.url}
        flagged = (
            _records.get_assistants("openai", aget_assistants=True, **args),
            _records.create_assistants(
                "openai", "gpt-4o-mini", async_create_assistants=True, **args),
            _records.delete_assistant(
                "openai", ASSISTANT_ID, async_delete_assistants=True, **args),
            _records.create_thread(
                "openai", messages=[{"role": "user", "content": "hi"}],
                acreate_thread=True, **args),
            _records.get_thread("openai", THREAD_ID, aget_thread=True, **args),
        )
        for answer in flagged:
            match answer:
                case Coroutine():
                    answer.close()
                case object():
                    raise AssertionError("the flag did not answer a coroutine")
        page = asyncio.run(_records.aget_assistants("openai", **args))
        assert page.has_more is False
        thread = asyncio.run(_records.acreate_thread(
            "openai", messages=[{"role": "user", "content": "hi"}], **args))
        assert thread is not None
        assert thread.id == THREAD_ID
        number_flag = _records.create_thread(
            "openai", messages=[{"role": "user", "content": "hi"}], acreate_thread=1, **args)
        assert not inspect.iscoroutine(number_flag)


def test_error_class_and_client() -> None:
    with AssistantServer() as server:
        good = {"api_key": GOOD_KEY, "api_base": server.url}
        bad = {"api_key": BAD_KEY, "api_base": server.url}
        with pytest.raises(errors.AuthenticationError) as ours:
            _records.get_assistants("openai", **bad)
        with pytest.raises(openai.AuthenticationError) as reference:
            LITELLM.get_assistants("openai", **bad)
        assert type(ours.value).__name__ == type(reference.value).__name__
        assert ours.value.status_code == UNAUTHORIZED_STATUS
        assert ours.value.model == ""
        assert "Incorrect API key" in str(ours.value)
        assert "Incorrect API key" in str(reference.value)
        with pytest.raises(errors.BadRequestError) as ours_unsupported:
            _records.get_assistants("bedrock", **good)
        with pytest.raises(THEIR_ERRORS.BadRequestError) as reference_unsupported:
            LITELLM.get_assistants("bedrock", **good)
        assert str(ours_unsupported.value) == str(reference_unsupported.value)
        assert ours_unsupported.value.model == "n/a"
        assert ours_unsupported.value.status_code == BAD_REQUEST_STATUS
        with pytest.raises(UnsupportedCallArgumentError, match="client"):
            _records.get_assistants("openai", client=FAKE_CLIENT, **good)
        with pytest.raises(ValueError, match="async_create_assistants"):
            _records.create_assistants("openai", "gpt-4o-mini", async_create_assistants=1)
        with pytest.raises(ValueError, match="async_delete_assistants"):
            _records.delete_assistant("openai", ASSISTANT_ID, async_delete_assistants="yes")
        with pytest.raises(Exception, match="aget_assistants"):
            _records.get_assistants("openai", aget_assistants=1)
