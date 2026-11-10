import asyncio
import inspect
import json
from collections.abc import AsyncIterator, Callable, Iterator
from contextlib import ExitStack
from typing import TypeAlias, cast
from unittest import mock

import httpx
import litellm
import openai
from litellm.llms.openai.common_utils import BaseOpenAILLM, OpenAIError
from litellm.llms.openai.completion import handler as handler_module
from litellm.llms.openai.completion.handler import OpenAITextCompletion
from litellm.types.utils import ModelResponse
from openai.types import Completion, CompletionChoice
from pydantic import BaseModel

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected object")
    return value


def text_map(value: Json) -> dict[str, str]:
    return {name: str(entry) for name, entry in obj(value).items()}


def failure_for(fields: Object) -> Exception:
    message = str(fields["message"])
    status = fields["status"]
    if not isinstance(status, int):
        return Exception(message)
    request = httpx.Request("POST", "https://api.openai.com/v1/completions")
    response = httpx.Response(status, headers=text_map(fields.get("headers", {})), request=request)
    return openai.APIStatusError(message, response=response, body=None)


class Raw:
    def __init__(self, parsed: object) -> None:
        self.parsed = parsed

    def parse(self) -> object:
        return self.parsed


def stream_chunk(item: Json) -> Completion:
    fields = obj(item)
    raw_choices = cast(list[Json], fields["choices"])
    choices = [CompletionChoice.model_construct(**obj(choice)) for choice in raw_choices]  # pyright: ignore[reportArgumentType]
    return Completion.model_construct(**{**fields, "choices": choices})  # pyright: ignore[reportArgumentType]


def sync_chunks(items: list[Json], failure: Exception | None) -> Iterator[Completion]:
    for item in items:
        yield stream_chunk(item)
    if failure is not None:
        raise failure


async def async_chunks(items: list[Json], failure: Exception | None) -> AsyncIterator[Completion]:
    for item in items:
        yield stream_chunk(item)
    if failure is not None:
        raise failure


class Fixture:
    def __init__(self, answer: Object) -> None:
        self.answer = answer
        self.calls: list[Json] = []
        self.built: list[Json] = []
        self.pre_call_records: list[Json] = []
        self.post_call_records: list[Json] = []

    def outcome(self, kwargs: Object, *, is_async: bool) -> Raw:
        self.calls.append(kwargs)
        if "failure" in self.answer:
            raise failure_for(obj(self.answer["failure"]))
        if "buffered" in self.answer:
            return Raw(Completion.model_validate(self.answer["buffered"]))
        stream = obj(self.answer["stream"])
        items = stream["chunks"]
        if not isinstance(items, list):
            raise TypeError("chunks must be a list")
        failure_fields = stream.get("failure")
        failure = None if failure_fields is None else failure_for(obj(failure_fields))
        return Raw(async_chunks(items, failure) if is_async else sync_chunks(items, failure))


class SyncCompletions:
    def __init__(self, fixture: Fixture) -> None:
        self.fixture = fixture

    @property
    def with_raw_response(self) -> "SyncCompletions":
        return self

    def create(self, **kwargs: Json) -> Raw:
        return self.fixture.outcome(kwargs, is_async=False)


class AsyncCompletions:
    def __init__(self, fixture: Fixture) -> None:
        self.fixture = fixture

    @property
    def with_raw_response(self) -> "AsyncCompletions":
        return self

    async def create(self, **kwargs: Json) -> Raw:
        return self.fixture.outcome(kwargs, is_async=True)


class Client:
    def __init__(self, completions: SyncCompletions | AsyncCompletions) -> None:
        self.completions = completions


class Factory:
    def __init__(self, fixture: Fixture, *, is_async: bool) -> None:
        self.fixture = fixture
        self.is_async = is_async

    def __call__(self, **kwargs: Json) -> Client:
        kind = "async" if self.is_async else "sync"
        self.fixture.built.append({"kind": kind, **kwargs})
        return self.client()

    def client(self) -> Client:
        if self.is_async:
            return Client(AsyncCompletions(self.fixture))
        return Client(SyncCompletions(self.fixture))


class Recorder:
    model_call_details: Object = {}  # noqa: RUF012
    optional_params: Object = {}  # noqa: RUF012
    stream_options: Json = None
    messages: Json = None
    completion_start_time: Json = None
    _llm_caching_handler: None = None
    call_type = "text_completion"

    def __init__(self, fixture: Fixture) -> None:
        self.fixture = fixture

    def __getattr__(self, name: str) -> Callable[..., None]:
        return self.ignore

    def ignore(self, *_args: object, **_kwargs: object) -> None:
        return None

    async def dispatch_failure_handlers(self, *_args: object, **_kwargs: object) -> None:
        return None

    def pre_call(self, input: Json, api_key: str | None, additional_args: Object) -> None:  # noqa: A002
        record: Object = {"input": input, "api_key": api_key, **additional_args}
        self.fixture.pre_call_records.append(record)

    def post_call(
        self, original_response: Json | BaseModel, api_key: str | None, additional_args: Object
    ) -> None:
        is_object = isinstance(original_response, BaseModel)
        dumped = cast(Json, original_response.model_dump()) if is_object else original_response
        record: Object = {
            "api_key": api_key,
            "original_response": dumped,
            "is_sdk_object": is_object,
            **additional_args,
        }
        self.fixture.post_call_records.append(record)


def described(error: Exception) -> Json:
    headers = cast(httpx.Headers | dict[str, str] | None, getattr(error, "headers", None))
    response = cast(httpx.Response | None, getattr(error, "response", None))
    if headers is None and response is not None:
        headers = response.headers
    fields = {str(name).lower(): str(value) for name, value in (headers or {}).items()}
    return {
        "mapped": isinstance(error, OpenAIError),
        "status": cast(Json, getattr(error, "status_code", None)),
        "message": cast(Json, getattr(error, "message", str(error))),
        "headers": cast(Json, fields) or None,
    }


def async_http_marker(shared_session: object = None) -> str:  # noqa: ARG001
    return "async_http_client"


def project(chunk: object) -> Json:
    dumped = cast(Object, cast(BaseModel, chunk).model_dump())
    choices = cast(list[Object], dumped["choices"])
    delta = obj(choices[0]["delta"]) if choices else {}
    return {
        "model": dumped["model"],
        "content": delta.get("content"),
        "finish_reason": choices[0]["finish_reason"] if choices else None,
    }


async def collect_async(stream: AsyncIterator[object], chunks: list[Json]) -> None:
    async for chunk in stream:
        chunks.append(project(chunk))  # noqa: PERF401


def collect(outcome: object, output: Object) -> None:
    if inspect.iscoroutine(outcome):
        outcome = cast(object, asyncio.run(outcome))
    chunks: list[Json] = []
    if isinstance(outcome, BaseModel):
        output["result"] = cast(Json, outcome.model_dump())
        hidden = cast(object, getattr(outcome, "_hidden_params"))  # noqa: B009
        original = cast(str | None, getattr(hidden, "original_response", None))
        # json.dumps spacing and key order are Python habits, so the oracle compares the parsed text
        output["hidden_original_response"] = None if original is None else json.loads(original)
        return
    output["chunks"] = chunks
    if inspect.isasyncgen(outcome):
        asyncio.run(collect_async(outcome, chunks))
    else:
        chunks.extend(project(chunk) for chunk in cast(Iterator[object], outcome))


def run(input_value: dict[str, Json]) -> Json:
    fixture = Fixture(obj(input_value["answer"]))
    request = obj(input_value["request"])
    is_async = request["acompletion"] is True
    output: Object = {"result": None, "hidden_original_response": None, "chunks": None}
    output["raised"] = None
    with ExitStack() as patches:
        patches.enter_context(
            mock.patch.object(handler_module, "OpenAI", Factory(fixture, is_async=False))
        )
        patches.enter_context(
            mock.patch.object(handler_module, "AsyncOpenAI", Factory(fixture, is_async=True))
        )
        patches.enter_context(mock.patch.object(litellm, "client_session", "client_session"))
        patches.enter_context(mock.patch.object(litellm, "aclient_session", "a_client_session"))
        patches.enter_context(
            mock.patch.object(
                BaseOpenAILLM,
                "_get_async_http_client",
                staticmethod(async_http_marker),
            )
        )
        supplied = input_value.get("supplied") is True
        client = Factory(fixture, is_async=is_async).client() if supplied else None
        try:
            collect(
                OpenAITextCompletion().completion(  # pyright: ignore[reportUnknownMemberType]
                    model_response=ModelResponse(),
                    api_key=cast(str, request.get("api_key")),
                    model=cast(str, request.get("model")),
                    messages=cast(list[Json], request.get("messages")),  # pyright: ignore[reportArgumentType]
                    timeout=cast(float, request["timeout"]),
                    custom_llm_provider=cast(str, input_value["provider"]),
                    logging_obj=cast(object, Recorder(fixture)),  # pyright: ignore[reportArgumentType]
                    optional_params=obj(request.get("optional_params", {})),
                    api_base=cast(str | None, request.get("api_base")),
                    acompletion=is_async,
                    client=client,
                    organization=cast(str | None, request.get("organization")),
                    headers=cast(dict[str, str] | None, request.get("headers")),
                ),
                output,
            )
        except Exception as error:  # noqa: BLE001
            output["raised"] = described(error)
    output["pre_call"] = fixture.pre_call_records
    output["post_call"] = fixture.post_call_records
    output["built"] = fixture.built
    output["calls"] = fixture.calls
    return output
