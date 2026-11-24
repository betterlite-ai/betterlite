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
    FAILING_KEY,
    GOOD_KEY,
    RATE_LIMITED_KEY,
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
RATE_LIMIT_STATUS: Final = 429
SERVER_ERROR_STATUS: Final = 500
RATE_LIMIT_TYPE: Final = "requests"
RATE_LIMIT_CODE: Final = "rate_limit_exceeded"
SERVER_ERROR_TYPE: Final = "server_error"
FAKE_CLIENT: Final = "openai.OpenAI"
COMPARED_HEADERS: Final[tuple[str, ...]] = ("authorization", "openai-beta")
CREATE: Final[dict[str, JsonValue]] = {
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
THREAD: Final[dict[str, JsonValue]] = {
    "messages": [{"role": "user", "content": "hi"}],
    "metadata": {"team": "a"},
    "tool_resources": {"code_interpreter": {"file_ids": ["file_1"]}},
}
TUPLE_MESSAGES: Final[tuple[CallValue, ...]] = ({"role": "user", "content": "hi"},)


@runtime_checkable
class Dumpable(Protocol):
    def model_dump_json(self) -> str: ...


@runtime_checkable
class Page(Protocol):
    data: Sequence[Dumpable]
    has_more: bool


Answer: TypeAlias = Dumpable | Page | Coroutine[None, None, Dumpable | Page | None] | None
RefResult: TypeAlias = Dumpable | Page | None
CallArgument: TypeAlias = JsonValue | tuple[CallValue, ...]


class SyncCall(Protocol):
    def __call__(self, *args: CallArgument, **kwargs: CallArgument) -> RefResult: ...


class AsyncCall(Protocol):
    def __call__(
        self, *args: CallArgument, **kwargs: CallArgument
    ) -> Coroutine[None, None, RefResult]: ...


class LiteLLMReference(Protocol):
    def __getattr__(self, name: str) -> SyncCall: ...


class LiteLLMAsyncReference(Protocol):
    def __getattr__(self, name: str) -> AsyncCall: ...


class LiteLLMErrors(Protocol):
    def __getattr__(self, name: str) -> type[Exception]: ...


LITELLM: LiteLLMReference = importlib.import_module("litellm")
LITELLM_ASYNC: LiteLLMAsyncReference = importlib.import_module("litellm")
THEIR_ERRORS: LiteLLMErrors = importlib.import_module("litellm.exceptions")


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


def shape(signature: inspect.Signature) -> list[tuple[str, str, bool]]:
    rows: list[tuple[str, str, bool]] = []
    for name, parameter in signature.parameters.items():
        default: DefaultView = parameter
        rows.append((name, parameter.kind.name, default.default is None))
    return rows


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


def flagged_run(answer: Answer) -> Dumpable | Page | None:
    match answer:
        case Coroutine() as coroutine:
            return asyncio.run(coroutine)
        case object():
            raise AssertionError("the flag did not answer a coroutine")


def answer_id(answer: Answer) -> JsonValue:
    match JSON.validate_json(answer_of(answer).model_dump_json()):
        case dict() as fields:
            return fields.get("id")
        case object():
            raise AssertionError("the answer is not an object")


def body_messages(call: RecordedCall) -> JsonValue:
    match call.get("body"):
        case dict() as body:
            return body.get("messages")
        case object():
            raise AssertionError("the thread call carried no body")


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


def create_ours(server: AssistantServer, *, asynchronous: bool = False) -> Answer:
    return _records.create_assistants(
        "openai", model="gpt-4o-mini", name="helper", description="a helper",
        instructions="be brief", tools=[{"type": "code_interpreter"}],
        tool_resources={"code_interpreter": {"file_ids": ["file_1"]}},
        metadata={"team": "a"}, temperature=0.0, top_p=0.0, response_format="auto",
        async_create_assistants=asynchronous, api_key=GOOD_KEY, api_base=server.url,
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
        reference = page_of(LITELLM.get_assistants("openai", timeout=None, **args))
        ours = page_of(_records.get_assistants("openai", timeout=None, **args))
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
    with AssistantServer() as server:
        answer = _records.create_thread(
            "openai", messages=[Message(content="hi", role="user")],
            api_key=GOOD_KEY, api_base=server.url,
        )
        assert body_messages(server.calls.pop()) == [{"role": "user", "content": "hi"}]
        assert answer_id(answer) == THREAD_ID


def test_calls_tuple_messages() -> None:
    with AssistantServer() as server:
        args = {"api_key": GOOD_KEY, "api_base": server.url}
        reference = answer_of(LITELLM.create_thread("openai", messages=TUPLE_MESSAGES, **args))
        ours = answer_of(_records.create_thread("openai", messages=TUPLE_MESSAGES, **args))
        matched_calls(server)
        dump_pair(ours, reference)


def test_async_flag_coroutines() -> None:
    with AssistantServer() as server:
        args = {"api_key": GOOD_KEY, "api_base": server.url}
        reference_page = page_of(asyncio.run(LITELLM_ASYNC.aget_assistants("openai", **args)))
        ours_page = page_of(flagged_run(
            _records.get_assistants("openai", aget_assistants=True, **args)))
        matched_calls(server)
        assert page_view(ours_page) == page_view(reference_page)
        reference_created = answer_of(asyncio.run(
            LITELLM_ASYNC.acreate_assistants("openai", **CREATE, **args)))
        ours_created = answer_of(flagged_run(create_ours(server, asynchronous=True)))
        matched_calls(server)
        dump_pair(ours_created, reference_created)
        reference_deleted = answer_of(asyncio.run(
            LITELLM_ASYNC.adelete_assistant("openai", assistant_id=ASSISTANT_ID, **args)))
        ours_deleted = answer_of(flagged_run(_records.delete_assistant(
            "openai", ASSISTANT_ID, async_delete_assistants=True, **args)))
        matched_calls(server)
        dump_pair(ours_deleted, reference_deleted)
        reference_thread = answer_of(asyncio.run(
            LITELLM_ASYNC.acreate_thread("openai", **THREAD, **args)))
        ours_thread = answer_of(flagged_run(_records.create_thread(
            "openai", acreate_thread=True, **THREAD, **args)))
        matched_calls(server)
        dump_pair(ours_thread, reference_thread)
        reference_got = answer_of(asyncio.run(
            LITELLM_ASYNC.aget_thread("openai", THREAD_ID, **args)))
        ours_got = answer_of(flagged_run(_records.get_thread(
            "openai", THREAD_ID, aget_thread=True, **args)))
        matched_calls(server)
        dump_pair(ours_got, reference_got)
        bad = {"api_key": BAD_KEY, "api_base": server.url}
        with pytest.raises(errors.AuthenticationError) as ours:
            asyncio.run(_records.aget_assistants("openai", **bad))
        with pytest.raises(openai.AuthenticationError) as reference:
            asyncio.run(LITELLM_ASYNC.aget_assistants("openai", **bad))
        assert type(ours.value).__name__ == type(reference.value).__name__
        assert ours.value.status_code == UNAUTHORIZED_STATUS
        assert "Incorrect API key" in str(ours.value)
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
        assert reference.value.type == "invalid_request_error"
        assert reference.value.code == "invalid_api_key"
        assert reference.value.param is None
        assert ours.value.type is None
        assert ours.value.param is None
        assert ours.value.code is None
        with pytest.raises(errors.BadRequestError) as ours_unsupported:
            _records.get_assistants("bedrock", **good)
        with pytest.raises(THEIR_ERRORS.BadRequestError) as reference_unsupported:
            LITELLM.get_assistants("bedrock", **good)
        assert str(ours_unsupported.value) == str(reference_unsupported.value)
        assert ours_unsupported.value.model == "n/a"
        assert ours_unsupported.value.status_code == BAD_REQUEST_STATUS
        with pytest.raises(UnsupportedCallArgumentError, match="client"):
            _records.get_assistants("openai", client=FAKE_CLIENT, **good)
        with pytest.raises(UnsupportedCallArgumentError, match="api_version"):
            _records.get_assistants("openai", api_version=1, **good)
        with pytest.raises(ValueError, match="async_create_assistants"):
            _records.create_assistants("openai", "gpt-4o-mini", async_create_assistants=1)
        with pytest.raises(ValueError, match="async_delete_assistants"):
            _records.delete_assistant("openai", ASSISTANT_ID, async_delete_assistants="yes")
        with pytest.raises(Exception, match="aget_assistants"):
            _records.get_assistants("openai", aget_assistants=1)


def test_error_fields_429_and_500() -> None:
    with AssistantServer() as server:
        limited = {"api_key": RATE_LIMITED_KEY, "api_base": server.url, "max_retries": 0}
        with pytest.raises(errors.RateLimitError) as ours_limited:
            _records.get_assistants("openai", **limited)
        with pytest.raises(openai.RateLimitError) as reference_limited:
            LITELLM.get_assistants("openai", **limited)
        assert ours_limited.value.status_code == RATE_LIMIT_STATUS
        assert ours_limited.value.model == ""
        assert ours_limited.value.param is None
        assert ours_limited.value.type == "throttling_error"
        assert ours_limited.value.code == "429"
        assert reference_limited.value.status_code == RATE_LIMIT_STATUS
        assert reference_limited.value.param is None
        assert reference_limited.value.type == RATE_LIMIT_TYPE
        assert reference_limited.value.code == RATE_LIMIT_CODE
        failing = {"api_key": FAILING_KEY, "api_base": server.url, "max_retries": 0}
        with pytest.raises(errors.InternalServerError) as ours_failing:
            _records.get_assistants("openai", **failing)
        with pytest.raises(openai.InternalServerError) as reference_failing:
            LITELLM.get_assistants("openai", **failing)
        assert ours_failing.value.status_code == SERVER_ERROR_STATUS
        assert ours_failing.value.model == ""
        assert ours_failing.value.param is None
        assert ours_failing.value.type == "internal_server_error"
        assert reference_failing.value.status_code == SERVER_ERROR_STATUS
        assert reference_failing.value.type == SERVER_ERROR_TYPE
        assert reference_failing.value.code is None
