import asyncio
from collections.abc import Callable, Coroutine
from concurrent.futures import ThreadPoolExecutor
from functools import partial
from typing import Literal, TypeAlias, TypeVar

import litellm
import openai
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from openai.pagination import AsyncCursorPage, SyncCursorPage
from openai.types.beta import AssistantDeleted
from openai.types.beta.assistant import Assistant
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

SENTINEL = "$API_BASE"
T = TypeVar("T")

Supplied: TypeAlias = OpenAI | AsyncOpenAI | AzureOpenAI | AsyncAzureOpenAI

Page: TypeAlias = SyncCursorPage[Assistant] | AsyncCursorPage[Assistant]
Answer: TypeAlias = (
    Page | AssistantDeleted | Coroutine[None, None, SyncCursorPage[Assistant] | AssistantDeleted]
)

KwargValue: TypeAlias = JsonValue | Supplied


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class Params(Strict):
    api_key: str | None = None
    api_base: str | None = None
    api_version: str | None = None
    max_retries: int | None = None
    timeout: str | float | None = None
    request_timeout: float | None = None
    azure_ad_token: str | None = None


class SuppliedClient(Strict):
    kind: Literal["openai", "async_openai", "azure_openai", "async_azure_openai"]
    api_base: str | None = None
    api_key: str | None = None
    api_version: str | None = None


class Input(Strict):
    operation: Literal["list", "delete"]
    provider: str
    assistant_id: str | None = None
    entry: Literal["sync", "async"] = "sync"
    aget_assistants: JsonValue = None
    async_delete_assistants: JsonValue = None
    extra_body: dict[str, JsonValue] | None = None
    client: SuppliedClient | None = None
    after: str | None = None
    before: str | None = None
    limit: int | None = None
    order: str | None = None
    params: Params = Field(default_factory=Params)


class Globals(Strict):
    api_base: str | None = None
    api_key: str | None = None
    openai_key: str | None = None
    azure_key: str | None = None


class Run(Strict):
    module_globals: Globals = Field(default_factory=Globals, alias="globals")
    entries: Literal["both", "sync", "async"] = "both"
    invocation: Input = Field(alias="input")


class AssistantReadDeleteCall(Strict):
    runs: list[Run]


class AssistantView(BaseModel):
    identifier: str = Field(alias="id")
    object_name: str = Field(alias="object")
    created_at: int
    model: str
    name: str | None = None


class PageView(BaseModel):
    object_name: str = Field(alias="object")
    data: list[AssistantView]
    has_more: bool
    first_id: str | None = None
    last_id: str | None = None


class DeletedView(BaseModel):
    identifier: str = Field(alias="id")
    object_name: str = Field(alias="object")
    deleted: bool


class FailureView(BaseModel):
    class_name: str = Field(serialization_alias="class")
    status: int | None = None
    kind: str | None = Field(default=None, serialization_alias="type")
    param: str | None = None
    code: str | None = None
    message: str


class Mode(BaseModel):
    page: PageView | None = None
    deleted: DeletedView | None = None
    coroutine: bool | None = None
    failure: FailureView | None = None


class RunOutcome(BaseModel):
    sync: Mode | None = None
    asynchronous: Mode | None = Field(default=None, serialization_alias="async")


class Outcome(BaseModel):
    runs: list[RunOutcome]


LIST_SYNC: Callable[..., Answer] = litellm.get_assistants
LIST_ASYNC: Callable[..., Coroutine[None, None, Answer]] = litellm.aget_assistants
DELETE_SYNC: Callable[..., Answer] = litellm.delete_assistant
DELETE_ASYNC: Callable[..., Coroutine[None, None, Answer]] = litellm.adelete_assistant


class AssistantReadDeleteRequest(Strict):
    assistant_read_delete: AssistantReadDeleteCall

    def serialize(self, api_base: str) -> str:
        runs = [run_once(run, api_base) for run in self.assistant_read_delete.runs]
        return Outcome(runs=runs).model_dump_json(by_alias=True, exclude_none=True)


def run_once(run: Run, api_base: str) -> RunOutcome:
    set_globals(run.module_globals, api_base)
    client = supplied_client(run.invocation.client, api_base)
    sync = run_mode(run.invocation, client, api_base, "sync") if run.entries != "async" else None
    asynchronous = (
        run_mode(run.invocation, client, api_base, "async") if run.entries != "sync" else None
    )
    return RunOutcome(sync=sync, asynchronous=asynchronous)


def set_globals(module_globals: Globals, api_base: str) -> None:
    litellm.api_base = base_of(module_globals.api_base, api_base)
    litellm.api_key = module_globals.api_key
    litellm.openai_key = module_globals.openai_key
    litellm.azure_key = module_globals.azure_key


def base_of(value: str | None, api_base: str) -> str | None:
    if value == SENTINEL:
        return api_base
    return value


def supplied_client(spec: SuppliedClient | None, api_base: str) -> Supplied | None:
    if spec is None:
        return None
    endpoint = base_of(spec.api_base, api_base)
    if spec.kind == "openai":
        return OpenAI(api_key=spec.api_key, base_url=endpoint)
    if spec.kind == "async_openai":
        return AsyncOpenAI(api_key=spec.api_key, base_url=endpoint)
    if spec.kind == "azure_openai":
        return AzureOpenAI(
            api_key=spec.api_key, azure_endpoint=endpoint, api_version=spec.api_version
        )
    return AsyncAzureOpenAI(
        api_key=spec.api_key, azure_endpoint=endpoint, api_version=spec.api_version
    )


def method_kwargs(invocation: Input, api_base: str) -> dict[str, KwargValue]:
    params = invocation.params
    values: tuple[tuple[str, KwargValue], ...] = (
        ("api_key", params.api_key),
        ("api_version", params.api_version),
        ("max_retries", params.max_retries),
        ("request_timeout", params.request_timeout),
        ("azure_ad_token", params.azure_ad_token),
        ("timeout", params.timeout),
        ("after", invocation.after),
        ("before", invocation.before),
        ("limit", invocation.limit),
        ("order", invocation.order),
    )
    kwargs: dict[str, KwargValue] = {name: value for name, value in values if value is not None}
    if params.api_base is not None:
        kwargs["api_base"] = base_of(params.api_base, api_base)
    if "aget_assistants" in invocation.model_fields_set:
        kwargs["aget_assistants"] = invocation.aget_assistants
    if "async_delete_assistants" in invocation.model_fields_set:
        kwargs["async_delete_assistants"] = invocation.async_delete_assistants
    if "extra_body" in invocation.model_fields_set:
        kwargs["extra_body"] = invocation.extra_body
    return kwargs


def run_mode(
    invocation: Input, client: Supplied | None, api_base: str, mode: Literal["sync", "async"]
) -> Mode:
    kwargs: dict[str, KwargValue] = {
        "custom_llm_provider": invocation.provider,
        "client": client,
        **method_kwargs(invocation, api_base),
    }
    if invocation.operation == "delete":
        kwargs["assistant_id"] = invocation.assistant_id
    value, error = call_entry(invocation, kwargs, mode)
    if error is not None:
        return Mode(failure=failure_view(error))
    if value is None:
        raise TypeError("the call returned nothing")
    return value_mode(value)


def call_entry(
    invocation: Input, kwargs: dict[str, KwargValue], mode: Literal["sync", "async"]
) -> tuple[Answer | None, BaseException | None]:
    listing = invocation.operation == "list"
    if mode == "async":
        function = LIST_ASYNC if listing else DELETE_ASYNC
        return capture(partial(asyncio.run, function(**kwargs)))
    return capture(partial(LIST_SYNC if listing else DELETE_SYNC, **kwargs))


def capture(call: Callable[[], T]) -> tuple[T | None, BaseException | None]:
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(call)
        error = future.exception()
        if error is not None:
            return None, error
        return future.result(), None


def value_mode(value: Answer) -> Mode:
    if asyncio.iscoroutine(value):
        value.close()
        return Mode(coroutine=True)
    match value:
        case SyncCursorPage() | AsyncCursorPage():
            return Mode(page=TypeAdapter(PageView).validate_json(value.model_dump_json()))
        case AssistantDeleted():
            return Mode(deleted=TypeAdapter(DeletedView).validate_json(value.model_dump_json()))


def failure_view(error: BaseException) -> FailureView:
    status: int | None = None
    kind: str | None = None
    param: str | None = None
    code: str | None = None
    message = str(error)
    match error:
        case openai.APIStatusError():
            status = error.status_code
            kind = error.type
            param = error.param
            code = error.code
            message = error.message
        case openai.APIError():
            kind = error.type
            param = error.param
            code = error.code
            message = error.message
    return FailureView(
        class_name=type(error).__name__,
        status=status,
        kind=kind,
        param=param,
        code=code,
        message=message,
    )
