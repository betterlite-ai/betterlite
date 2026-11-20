import asyncio
from collections.abc import Coroutine
from concurrent.futures import ThreadPoolExecutor
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

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
    model_config = ConfigDict(extra="ignore")  # the wire carries Azure credential kwargs
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


class CallArgs(TypedDict):
    custom_llm_provider: Literal["openai", "azure"]
    client: AsyncOpenAI | None
    api_key: str | None
    api_base: str | None
    api_version: str | None
    max_retries: int | None
    request_timeout: float | None
    after: str | None
    before: str | None
    limit: int | None
    order: str | None
    aget_assistants: JsonValue
    async_delete_assistants: JsonValue
    timeout: NotRequired[str | float]
    extra_body: NotRequired[dict[str, JsonValue] | None]
    azure_ad_token: NotRequired[str]


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


class RaisedStatus(BaseModel):
    status_code: int | None = None


class Mode(BaseModel):
    page: PageView | None = None
    deleted: DeletedView | None = None
    coroutine: bool | None = None
    failure: FailureView | None = None


class RunOutcome(BaseModel):
    sync: Mode | None = None
    asynchronous: Mode | None = Field(default=None, serialization_alias="async")


class Outcome(BaseModel):
    assistant_read_delete: dict[Literal["value"], dict[Literal["runs"], list[RunOutcome]]]


class AssistantReadDeleteRequest(Strict):
    assistant_read_delete: AssistantReadDeleteCall

    def serialize(self, api_base: str) -> str:
        runs = [run_once(run, api_base) for run in self.assistant_read_delete.runs]
        outcome = Outcome(assistant_read_delete={"value": {"runs": runs}})
        return outcome.model_dump_json(by_alias=True, exclude_none=True)


def run_once(run: Run, api_base: str) -> RunOutcome:
    global_base = run.module_globals.api_base
    litellm.api_base = api_base if global_base == SENTINEL else global_base
    litellm.api_version = run.module_globals.api_version
    litellm.api_key = run.module_globals.api_key
    litellm.openai_key = run.module_globals.openai_key
    litellm.azure_key = run.module_globals.azure_key
    client = cast("AsyncOpenAI | None", supplied_client(run.invocation.client, api_base))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L38-L45
    sync = run_mode(run.invocation, client, api_base, "sync") if run.entries != "async" else None
    asynchronous = (
        run_mode(run.invocation, client, api_base, "async") if run.entries != "sync" else None
    )
    return RunOutcome(sync=sync, asynchronous=asynchronous)


def supplied_client(spec: SuppliedClient | None, api_base: str) -> Supplied | None:
    if spec is None:
        return None
    endpoint = api_base if spec.api_base == SENTINEL else spec.api_base
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
    args: CallArgs = {
        "custom_llm_provider": cast("Literal['openai', 'azure']", invocation.provider),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L73-L80
        "client": client,
        "api_key": params.api_key,
        "api_base": api_base if params.api_base == SENTINEL else params.api_base,
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
    if params.timeout is not None:
        args["timeout"] = params.timeout
    if params.azure_ad_token is not None:
        args["azure_ad_token"] = params.azure_ad_token
    if "extra_body" in invocation.model_fields_set:
        args["extra_body"] = invocation.extra_body
    return args


def required_assistant_id(invocation: Input) -> str:
    if invocation.assistant_id is None:
        raise TypeError("assistant_id is required for delete_assistant")
    return invocation.assistant_id


def run_mode(
    invocation: Input, client: AsyncOpenAI | None, api_base: str, mode: Literal["sync", "async"]
) -> Mode:
    entry = run_async if mode == "async" else entry_sync
    with ThreadPoolExecutor(max_workers=1) as executor:
        future = executor.submit(entry, invocation, client, api_base)
    error = future.exception()
    if error is not None:
        return Mode(failure=failure_view(error))
    return value_mode(future.result())


def entry_sync(invocation: Input, client: AsyncOpenAI | None, api_base: str) -> Answer:
    args = call_args(invocation, client, api_base)
    if invocation.operation == "list":
        page: SyncCursorPage[Assistant] = get_assistants(**args)
        return page
    return delete_assistant(**args, assistant_id=required_assistant_id(invocation))


async def entry_async(invocation: Input, client: AsyncOpenAI | None, api_base: str) -> Answer:
    args = call_args(invocation, client, api_base)
    if invocation.operation == "list":
        page: AsyncCursorPage[Assistant] = await aget_assistants(**args)
        return page
    return await adelete_assistant(**args, assistant_id=required_assistant_id(invocation))


def run_async(invocation: Input, client: AsyncOpenAI | None, api_base: str) -> Answer:
    with asyncio.Runner() as runner:
        return runner.run(entry_async(invocation, client, api_base))


def value_mode(value: Answer) -> Mode:
    match value:
        case SyncCursorPage() | AsyncCursorPage():
            return Mode(page=TypeAdapter(PageView).validate_json(value.model_dump_json()))
        case AssistantDeleted():
            return Mode(deleted=TypeAdapter(DeletedView).validate_json(value.model_dump_json()))
        case Coroutine():
            value.close()
            return Mode(coroutine=True)


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
                status=RaisedStatus.model_validate(error, from_attributes=True).status_code,
                kind=error.type,
                param=error.param,
                code=error.code,
                message=error.message,
            )
        case BaseException():
            return FailureView(class_name=type(error).__name__, message=str(error))
