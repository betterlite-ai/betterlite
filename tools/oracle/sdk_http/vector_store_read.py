import asyncio
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import ClassVar, Literal, Never, Protocol, TypeVar, TypedDict, Unpack

import litellm
import openai
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

from openai_chat import Failure, project_failure

Operation = Literal["retrieve", "list"]
Mode = Literal["sync", "async"]
Work = Coroutine[object, object, JsonValue]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PARAMS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
TEXT: TypeAdapter[str] = TypeAdapter(str)
Answer_co = TypeVar("Answer_co", covariant=True)


# every keyword argument the four reads take
class ReadValues(TypedDict, total=False):
    vector_store_id: str
    after: str | None
    before: str | None
    limit: int | None
    order: str | None
    api_base: str
    api_key: str | None
    litellm_call_id: str
    litellm_logging_obj: Logging
    custom_llm_provider: str | None
    extra_headers: dict[str, str] | None
    extra_query: dict[str, JsonValue] | None
    extra_body: dict[str, JsonValue] | None
    timeout: float | None


class ReadCall(Protocol[Answer_co]):
    def __call__(self, **values: Unpack[ReadValues]) -> Answer_co: ...


class Reads(Protocol[Answer_co]):
    def __getattr__(self, name: str) -> ReadCall[Answer_co]: ...


class VectorStores(Protocol[Answer_co]):
    def __getattr__(self, name: str) -> Reads[Answer_co]: ...


class LitellmModule(Protocol[Answer_co]):
    def __getattr__(self, name: str) -> VectorStores[Answer_co]: ...


# the @client decorator erases each read's signature, so a typed attribute view reaches the four
SYNC_LITELLM: LitellmModule[JsonValue] = litellm
ASYNC_LITELLM: LitellmModule[Work] = litellm
SYNC: Reads[JsonValue] = SYNC_LITELLM.vector_stores.main
ASYNC: Reads[Work] = ASYNC_LITELLM.vector_stores.main


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class RunOptions(Strict):
    api_key: str | None = None
    litellm_call_id: str


class Run(Strict):
    operation: Operation
    mode: Mode
    vector_store_id: str | None = None
    after: str | None = None
    before: str | None = None
    limit: int | None = None
    order: str | None = None
    options: RunOptions
    extra_headers: dict[str, str] | None = None
    extra_query: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    timeout: float | None = None
    custom_llm_provider: str | None = None


class Call(Strict):
    runs: list[Run]


class Context(Strict):
    model: str
    optional_params: dict[str, JsonValue]
    custom_llm_provider: str
    litellm_call_id: JsonValue


class RunOutcome(Strict):
    operation: Operation
    mode: Mode
    result: JsonValue = None
    failure: Failure | None = None
    context: Context | None = None


class Outcome(Strict):
    runs: list[RunOutcome]


class VectorStoreReadValue(Strict):
    value: Outcome


class VectorStoreReadResult(Strict):
    vector_store_read: VectorStoreReadValue


class VectorStoreReadRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_store_read: Call

    def serialize(self, api_base: str) -> str:
        held = litellm.vector_store_registry
        try:
            litellm.vector_store_registry = RefusingRegistry()
            runs = asyncio.run(run_all(self.vector_store_read.runs, api_base))
        finally:
            litellm.vector_store_registry = held
        value = VectorStoreReadValue(value=Outcome(runs=runs))
        return VectorStoreReadResult(vector_store_read=value).model_dump_json(by_alias=True)


# a read resolves its credential from the caller or the environment, never from the registry
class RefusingRegistry(VectorStoreRegistry):
    refusal: ClassVar[str] = "a vector store read must not consult the registry"

    @override
    def get_credentials_for_vector_store(  # ast-grep-ignore: no-long-names
        self, _vector_store_id: str
    ) -> Never:
        raise RuntimeError(self.refusal)


def require_id(run: Run) -> str:
    vector_store_id = run.vector_store_id
    if vector_store_id is None:
        raise ValueError("a retrieve run carries no vector_store_id")
    return vector_store_id


# an absent limit or order takes the signature's default; an explicit null stays null
def limit_value(run: Run) -> int | None:
    if "limit" in run.model_fields_set:
        return run.limit
    return 20


def order_value(run: Run) -> str | None:
    if "order" in run.model_fields_set:
        return run.order
    return "desc"


def read_options(run: Run, api_base: str, logging_obj: Logging) -> ReadValues:
    return ReadValues(
        api_base=api_base,
        api_key=run.options.api_key,
        litellm_call_id=run.options.litellm_call_id,
        litellm_logging_obj=logging_obj,
        custom_llm_provider=run.custom_llm_provider,
        extra_headers=run.extra_headers,
        extra_query=run.extra_query,
        extra_body=run.extra_body,
        timeout=run.timeout,
    )


def retrieve_options(run: Run, api_base: str, logging_obj: Logging) -> ReadValues:
    options = read_options(run, api_base, logging_obj)
    options["vector_store_id"] = require_id(run)
    return options


def list_options(run: Run, api_base: str, logging_obj: Logging) -> ReadValues:
    options = read_options(run, api_base, logging_obj)
    options["after"] = run.after
    options["before"] = run.before
    options["limit"] = limit_value(run)
    options["order"] = order_value(run)
    return options


def answer(run: Run, api_base: str, logging_obj: Logging) -> JsonValue:
    if run.operation == "retrieve":
        return JSON.validate_python(SYNC.retrieve(**retrieve_options(run, api_base, logging_obj)))
    return JSON.validate_python(SYNC.list(**list_options(run, api_base, logging_obj)))


async def async_answer(run: Run, api_base: str, logging_obj: Logging) -> JsonValue:
    if run.operation == "retrieve":
        return JSON.validate_python(
            await ASYNC.aretrieve(**retrieve_options(run, api_base, logging_obj))
        )
    return JSON.validate_python(await ASYNC.alist(**list_options(run, api_base, logging_obj)))


def logging_fixture(call_type: str) -> Logging:
    return Logging(
        model="vector-store",
        messages=None,
        stream=False,
        call_type=call_type,
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


# update_from_kwargs is what names the provider; a call that failed before it records no context
def context_of(details: dict[str, JsonValue]) -> Context | None:
    if "custom_llm_provider" not in details:
        return None
    params = PARAMS.validate_python(details.get("litellm_params"))
    return Context(
        model=TEXT.validate_python(details.get("model")),
        optional_params=PARAMS.validate_python(details.get("optional_params")),
        custom_llm_provider=TEXT.validate_python(details["custom_llm_provider"]),
        litellm_call_id=params.get("litellm_call_id"),
    )


async def run_once(run: Run, api_base: str) -> RunOutcome:
    logging_obj = logging_fixture(f"vector_store_{run.operation}")
    details: dict[str, JsonValue] = {}
    logging_obj.model_call_details = details
    try:
        if run.mode == "async":
            result = await async_answer(run, api_base, logging_obj)
        else:
            result = answer(run, api_base, logging_obj)
    except openai.APIError as error:
        return RunOutcome(
            operation=run.operation,
            mode=run.mode,
            result=None,
            failure=project_failure(error).failure,
            context=context_of(details),
        )
    return RunOutcome(
        operation=run.operation,
        mode=run.mode,
        result=result,
        failure=None,
        context=context_of(details),
    )


# one event loop for the whole case: LiteLLM caches its async client against the loop that made it
async def run_all(runs: list[Run], api_base: str) -> list[RunOutcome]:
    return [
        await run_once(run, api_base)
        for run in runs
    ]
