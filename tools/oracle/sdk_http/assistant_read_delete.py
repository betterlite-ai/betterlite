import asyncio
import types
from collections.abc import Callable, Coroutine
from concurrent.futures import ThreadPoolExecutor
from functools import partial
from typing import Any, Literal, TypeAlias, TypeVar

import litellm
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from openai.pagination import AsyncCursorPage, SyncCursorPage
from openai.types.beta import AssistantDeleted
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

MISSING = object()
SENTINEL = "$API_BASE"
T = TypeVar("T")


class Params(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_key: str | None = None
    api_base: str | None = None
    api_version: str | None = None
    max_retries: int | None = None
    timeout: str | float | None = None
    request_timeout: float | None = None
    azure_ad_token: str | None = None


class SuppliedClient(BaseModel):
    model_config = ConfigDict(extra="forbid")

    kind: Literal["openai", "async_openai", "azure_openai", "async_azure_openai"]
    api_base: str | None = None
    api_key: str | None = None
    api_version: str | None = None


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Literal["list", "delete"]
    provider: str
    assistant_id: str | None = None
    entry: Literal["sync", "async"] = "sync"
    aget_assistants: object = None
    async_delete_assistants: object = None
    extra_body: dict[str, JsonValue] | None = None
    client: SuppliedClient | None = None
    after: str | None = None
    before: str | None = None
    limit: int | None = None
    order: str | None = None
    params: Params = Field(default_factory=Params)


class Globals(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_base: str | None = None
    api_key: str | None = None
    openai_key: str | None = None
    azure_key: str | None = None


class Run(BaseModel):
    model_config = ConfigDict(extra="forbid")

    module_globals: Globals = Field(default_factory=Globals, alias="globals")
    entries: Literal["both", "sync", "async"] = "both"
    invocation: Input = Field(alias="input")


class AssistantReadDeleteCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

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


SyncFunction: TypeAlias = Callable[..., object]
AsyncFunction: TypeAlias = Callable[..., Coroutine[Any, Any, object]]

LIST_SYNC: SyncFunction = litellm.get_assistants
LIST_ASYNC: AsyncFunction = litellm.aget_assistants
DELETE_SYNC: SyncFunction = litellm.delete_assistant
DELETE_ASYNC: AsyncFunction = litellm.adelete_assistant

PAGE = TypeAdapter(PageView)
DELETED = TypeAdapter(DeletedView)


class AssistantReadDeleteRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    assistant_read_delete: AssistantReadDeleteCall

    def serialize(self, api_base: str) -> str:
        runs = [run_once(run, api_base) for run in self.assistant_read_delete.runs]
        return Outcome(runs=runs).model_dump_json(by_alias=True, exclude_none=True)


def run_once(run: Run, api_base: str) -> RunOutcome:
    set_globals(run.module_globals, api_base)
    client = supplied_client(run.invocation.client, api_base)
    return RunOutcome(
        sync=run_mode(run.invocation, client, api_base, "sync")
        if run.entries != "async"
        else None,
        asynchronous=run_mode(run.invocation, client, api_base, "async")
        if run.entries != "sync"
        else None,
    )


def set_globals(globals: Globals, api_base: str) -> None:
    litellm.api_base = base_of(globals.api_base, api_base)
    litellm.api_key = globals.api_key
    litellm.openai_key = globals.openai_key
    litellm.azure_key = globals.azure_key


def base_of(value: str | None, api_base: str) -> str | None:
    return api_base if value == SENTINEL else value


def supplied_client(spec: SuppliedClient | None, api_base: str) -> object:
    if spec is None:
        return None
    endpoint = base_of(spec.api_base, api_base)
    match spec.kind:
        case "openai":
            return OpenAI(api_key=spec.api_key, base_url=endpoint)
        case "async_openai":
            return AsyncOpenAI(api_key=spec.api_key, base_url=endpoint)
        case "azure_openai":
            return AzureOpenAI(
                api_key=spec.api_key, azure_endpoint=endpoint, api_version=spec.api_version
            )
        case "async_azure_openai":
            return AsyncAzureOpenAI(
                api_key=spec.api_key, azure_endpoint=endpoint, api_version=spec.api_version
            )


def method_kwargs(invocation: Input, api_base: str) -> dict[str, object]:
    params = invocation.params
    values: tuple[tuple[str, object], ...] = (
        ("api_key", params.api_key),
        ("api_version", params.api_version),
        ("max_retries", params.max_retries),
        ("request_timeout", params.request_timeout),
        ("azure_ad_token", params.azure_ad_token),
    )
    kwargs: dict[str, object] = {name: value for name, value in values if value is not None}
    if params.api_base is not None:
        kwargs["api_base"] = base_of(params.api_base, api_base)
    if params.timeout is not None:
        kwargs["timeout"] = params.timeout
    if "aget_assistants" in invocation.model_fields_set:
        kwargs["aget_assistants"] = invocation.aget_assistants
    if "async_delete_assistants" in invocation.model_fields_set:
        kwargs["async_delete_assistants"] = invocation.async_delete_assistants
    if "extra_body" in invocation.model_fields_set:
        kwargs["extra_body"] = invocation.extra_body
    return kwargs


def run_mode(
    invocation: Input, client: object, api_base: str, mode: Literal["sync", "async"]
) -> Mode:
    kwargs: dict[str, object] = {
        "custom_llm_provider": invocation.provider,
        "client": client,
        **method_kwargs(invocation, api_base),
    }
    if invocation.operation == "delete":
        kwargs["assistant_id"] = invocation.assistant_id
    value, error = call_entry(invocation, kwargs, mode)
    if error is not None:
        return Mode(failure=failure_view(error))
    if isinstance(value, types.CoroutineType):
        value.close()
        return Mode(coroutine=True)
    return value_mode(value)


def call_entry(
    invocation: Input, kwargs: dict[str, object], mode: Literal["sync", "async"]
) -> tuple[object | None, BaseException | None]:
    listing = invocation.operation == "list"
    if mode == "async":
        return capture(partial(invoke_async, LIST_ASYNC if listing else DELETE_ASYNC, kwargs))
    return capture(partial(invoke, LIST_SYNC if listing else DELETE_SYNC, kwargs))


def invoke(function: Callable[..., object], kwargs: dict[str, object]) -> object:
    return function(**kwargs)


def invoke_async(
    function: Callable[..., Coroutine[Any, Any, object]], kwargs: dict[str, object]
) -> object:
    return asyncio.run(function(**kwargs))


def capture(call: Callable[[], T]) -> tuple[T | None, BaseException | None]:
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(call)
        error = future.exception()
        if error is not None:
            return None, error
        return future.result(), None


def value_mode(value: object) -> Mode:
    if isinstance(value, (SyncCursorPage, AsyncCursorPage)):
        return Mode(page=PAGE.validate_json(value.model_dump_json()))
    if isinstance(value, AssistantDeleted):
        return Mode(deleted=DELETED.validate_json(value.model_dump_json()))
    raise TypeError(f"the call returned an unprojected {type(value).__name__}")


def failure_view(error: BaseException) -> FailureView:
    return FailureView(
        class_name=type(error).__name__,
        status=status_field(error),
        kind=text_field(error, "type"),
        param=text_field(error, "param"),
        code=text_field(error, "code"),
        message=str(error),
    )


def text_field(error: BaseException, name: str) -> str | None:
    value: object = getattr(error, name, MISSING)
    if isinstance(value, str):
        return value
    return None


def status_field(error: BaseException) -> int | None:
    value: object = getattr(error, "status_code", MISSING)
    if isinstance(value, int):
        return value
    return None
