import asyncio
from contextlib import ExitStack
from typing import TypeAlias, TypedDict, cast
from unittest import mock

import httpx
import litellm.llms.openai.openai as openai_module
import openai
from litellm.llms.openai.openai import OpenAIBatchesAPI
from litellm.types.llms.openai import CancelBatchRequest, CreateBatchRequest, RetrieveBatchRequest
from openai.types import Batch
from pydantic import BaseModel

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected object")
    return value


def failure_for(fields: Object) -> openai.APIStatusError:
    status = fields["status"]
    message = fields["message"]
    if not isinstance(status, int) or not isinstance(message, str):
        raise TypeError("failure needs a status and a message")
    request = httpx.Request("POST", "https://api.openai.com/v1/batches")
    body: Object = {name: fields.get(name) for name in ("type", "param", "code")}
    return openai.APIStatusError(
        message, response=httpx.Response(status, request=request), body=body
    )


class Fixture:
    def __init__(self, answer: Object) -> None:
        self.answer = answer
        self.calls: list[Json] = []
        self.built: list[Json] = []
        self.sync_class = sync_client_class(self)
        self.async_class = async_client_class(self)

    def reply(self, method: str, kwargs: Object) -> object:
        self.calls.append({"method": method, "kwargs": kwargs})
        if "failure" in self.answer:
            raise failure_for(obj(self.answer["failure"]))
        value = self.answer["value"]
        return value if method == "list" else Batch.model_validate(value)


def sync_client_class(fixture: Fixture) -> type[openai.OpenAI]:
    class Batches:
        def create(self, **kwargs: Json) -> object:
            return fixture.reply("create", kwargs)

        def retrieve(self, **kwargs: Json) -> object:
            return fixture.reply("retrieve", kwargs)

        def cancel(self, **kwargs: Json) -> object:
            return fixture.reply("cancel", kwargs)

        def list(self, after: str | None, limit: int | None) -> object:
            return fixture.reply("list", {"after": after, "limit": limit})

    class SyncClient(openai.OpenAI):
        def __init__(self, **kwargs: Json) -> None:
            fixture.built.append({"kind": "sync", "kwargs": kwargs})
            super().__init__(api_key="fixture-key")
            self.__dict__["batches"] = Batches()

    return SyncClient


def async_client_class(fixture: Fixture) -> type[openai.AsyncOpenAI]:
    class Batches:
        async def create(self, **kwargs: Json) -> object:
            return fixture.reply("create", kwargs)

        async def retrieve(self, **kwargs: Json) -> object:
            return fixture.reply("retrieve", kwargs)

        async def cancel(self, **kwargs: Json) -> object:
            return fixture.reply("cancel", kwargs)

        async def list(self, after: str | None, limit: int | None) -> object:
            return fixture.reply("list", {"after": after, "limit": limit})

    class AsyncClient(openai.AsyncOpenAI):
        def __init__(self, **kwargs: Json) -> None:
            fixture.built.append({"kind": "async", "kwargs": kwargs})
            super().__init__(api_key="fixture-key")
            self.__dict__["batches"] = Batches()

    return AsyncClient


class Common(TypedDict):
    api_key: str | None
    api_base: str | None
    timeout: float
    max_retries: int | None
    organization: str | None


def invoke(
    api: OpenAIBatchesAPI,
    request: Object,
    settings: Common,
    *,
    is_async: bool,
    client: openai.OpenAI | openai.AsyncOpenAI | None,
) -> object:
    ((method, params),) = request.items()
    sync_client = cast(openai.OpenAI | None, client)
    if method == "create":
        outcome = api.create_batch(
            _is_async=is_async,
            create_batch_data=cast(CreateBatchRequest, obj(params)),
            client=client,
            **settings,
        )
    elif method == "retrieve":
        outcome = api.retrieve_batch(
            _is_async=is_async,
            retrieve_batch_data=cast(RetrieveBatchRequest, obj(params)),
            client=sync_client,
            **settings,
        )
    elif method == "cancel":
        outcome = api.cancel_batch(
            _is_async=is_async,
            cancel_batch_data=cast(CancelBatchRequest, obj(params)),
            client=sync_client,
            **settings,
        )
    else:
        listing = obj(params)
        outcome = api.list_batches(
            _is_async=is_async,
            after=cast(str | None, listing["after"]),
            limit=cast(int | None, listing["limit"]),
            client=sync_client,
            **settings,
        )
    return asyncio.run(outcome) if asyncio.iscoroutine(outcome) else outcome


def run(input_value: dict[str, Json]) -> Json:
    fixture = Fixture(obj(input_value["answer"]))
    with ExitStack() as patches:
        patches.enter_context(mock.patch.object(openai_module, "OpenAI", fixture.sync_class))
        patches.enter_context(mock.patch.object(openai_module, "AsyncOpenAI", fixture.async_class))
        if input_value.get("unbuildable") is True:
            patches.enter_context(
                mock.patch.object(OpenAIBatchesAPI, "get_openai_client", return_value=None)
            )
        supplied = input_value.get("supplied")
        client: openai.OpenAI | openai.AsyncOpenAI | None = None
        if supplied is not None:
            client = fixture.async_class() if supplied == "async" else fixture.sync_class()
        fixture.built.clear()
        result: Json = None
        raised: Json = None
        try:
            reply = invoke(
                OpenAIBatchesAPI(),
                obj(input_value["request"]),
                cast(Common, obj(input_value["settings"])),
                is_async=input_value["is_async"] is True,
                client=client,
            )
        except openai.APIStatusError as error:
            raised = {
                "status": error.status_code,
                "type": error.type,
                "param": error.param,
                "code": error.code,
                "message": error.message,
            }
        else:
            result = (
                reply.model_dump(mode="json") if isinstance(reply, BaseModel) else cast(Json, reply)
            )
    return {"result": result, "raised": raised, "calls": fixture.calls, "built": fixture.built}
