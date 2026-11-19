import asyncio
from collections.abc import Coroutine
from concurrent.futures import ThreadPoolExecutor
from typing import Literal, TypeAlias, TypedDict, cast

import litellm
import openai
from litellm import adelete_assistant, aget_assistants, delete_assistant, get_assistants  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L38-L73
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from openai.pagination import AsyncCursorPage, SyncCursorPage
from openai.types.beta import AssistantDeleted
from openai.types.beta.assistant import Assistant
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

SENTINEL = "$API_BASE"

Supplied: TypeAlias = OpenAI | AsyncOpenAI | AzureOpenAI | AsyncAzureOpenAI

Page: TypeAlias = SyncCursorPage[Assistant] | AsyncCursorPage[Assistant]
Answer: TypeAlias = Page | AssistantDeleted | Coroutine[None, None, Page | AssistantDeleted]


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class Params(Strict):
    # the wire also carries the Azure credential kwargs, which the mocked entry points never take
    model_config = ConfigDict(extra="ignore")
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
    api_version: str | None = None
    api_key: str | None = None
    openai_key: str | None = None
    azure_key: str | None = None


class Run(Strict):
    module_globals: Globals = Field(default_factory=Globals, alias="globals")
    entries: Literal["both", "sync", "async"] = "both"
    invocation: Input = Field(alias="input")


class AssistantReadDeleteCall(Strict):
    runs: list[Run]


class CallArgs(TypedDict, total=False):
    custom_llm_provider: Literal["openai", "azure"]
    client: AsyncOpenAI | None
    assistant_id: str
    api_key: str | None
    api_base: str | None
    api_version: str | None
    max_retries: int | None
    timeout: str | float
    request_timeout: float | None
    azure_ad_token: str
    after: str | None
    before: str | None
    limit: int | None
    order: str | None
    extra_body: dict[str, JsonValue] | None
    aget_assistants: JsonValue
    async_delete_assistants: JsonValue


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


class AssistantReadDeleteRequest(Strict):
    assistant_read_delete: AssistantReadDeleteCall

    def serialize(self, api_base: str) -> str:
        runs = [run_once(run, api_base) for run in self.assistant_read_delete.runs]
        return Outcome(runs=runs).model_dump_json(by_alias=True, exclude_none=True)


def run_once(run: Run, api_base: str) -> RunOutcome:
    litellm.api_base = base_of(run.module_globals.api_base, api_base)
    litellm.api_version = run.module_globals.api_version
    litellm.api_key = run.module_globals.api_key
    litellm.openai_key = run.module_globals.openai_key
    litellm.azure_key = run.module_globals.azure_key
    supplied = supplied_client(run.invocation.client, api_base)
    client: AsyncOpenAI | None = cast("AsyncOpenAI | None", supplied)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L38-L45
    sync = run_mode(run.invocation, client, api_base, "sync") if run.entries != "async" else None
    asynchronous = (
        run_mode(run.invocation, client, api_base, "async") if run.entries != "sync" else None
    )
    return RunOutcome(sync=sync, asynchronous=asynchronous)


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
    azure_endpoint = cast("str", endpoint)  # https://github.com/openai/openai-python/blob/03b3ec474241fe3a73c4818dc6b886da28faaf91/src/openai/lib/azure.py#L169-L173
    if spec.kind == "azure_openai":
        return AzureOpenAI(
            api_key=spec.api_key, azure_endpoint=azure_endpoint, api_version=spec.api_version
        )
    return AsyncAzureOpenAI(
        api_key=spec.api_key, azure_endpoint=azure_endpoint, api_version=spec.api_version
    )


def call_args(invocation: Input, client: AsyncOpenAI | None, api_base: str) -> CallArgs:
    params = invocation.params
    # the case names any provider, and LiteLLM's wrappers reject the rest themselves
    args: CallArgs = {
        "custom_llm_provider": cast("Literal['openai', 'azure']", invocation.provider),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L73-L80
        "client": client,
        "api_key": params.api_key,
        "api_base": base_of(params.api_base, api_base),
        "api_version": params.api_version,
        "max_retries": params.max_retries,
        "request_timeout": params.request_timeout,
        "after": invocation.after,
        "before": invocation.before,
        "limit": invocation.limit,
        "order": invocation.order,
        "aget_assistants": invocation.aget_assistants,
        "async_delete_assistants": invocation.async_delete_assistants,
    }
    if invocation.assistant_id is not None:
        args["assistant_id"] = invocation.assistant_id
    if params.timeout is not None:
        args["timeout"] = params.timeout
    if params.azure_ad_token is not None:
        args["azure_ad_token"] = params.azure_ad_token
    if "extra_body" in invocation.model_fields_set:
        args["extra_body"] = invocation.extra_body
    return args


def run_mode(
    invocation: Input, client: AsyncOpenAI | None, api_base: str, mode: Literal["sync", "async"]
) -> Mode:
    args = call_args(invocation, client, api_base)
    entry = run_async if mode == "async" else entry_sync
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(entry, invocation, args)
    error = future.exception()
    if error is not None:
        return Mode(failure=failure_view(error))
    return value_mode(future.result())


def entry_sync(invocation: Input, args: CallArgs) -> Answer:
    if invocation.operation == "list":
        page: SyncCursorPage[Assistant] = get_assistants(**args)
        return page
    deleted: AssistantDeleted | Coroutine[None, None, AssistantDeleted] = delete_assistant(**args)
    return deleted


async def entry_async(invocation: Input, args: CallArgs) -> Answer:
    if invocation.operation == "list":
        page: AsyncCursorPage[Assistant] = await aget_assistants(**args)
        return page
    deleted: AssistantDeleted = await adelete_assistant(**args)
    return deleted


def run_async(invocation: Input, args: CallArgs) -> Answer:
    with asyncio.Runner() as runner:
        return runner.run(entry_async(invocation, args))


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
    match error:
        case openai.APIStatusError():
            return FailureView(
                class_name=type(error).__name__,
                status=error.status_code,
                kind=error.type,
                param=error.param,
                code=error.code,
                message=error.message,
            )
        case openai.APIError():
            return FailureView(
                class_name=type(error).__name__,
                kind=error.type,
                param=error.param,
                code=error.code,
                message=error.message,
            )
        case BaseException():
            return FailureView(class_name=type(error).__name__, message=str(error))
