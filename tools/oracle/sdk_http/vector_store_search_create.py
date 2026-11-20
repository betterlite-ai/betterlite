import asyncio
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import Literal, Protocol, TypeVar, TypedDict, Unpack

import litellm
import openai
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.models.credentials import CredentialItem
from litellm.types.vector_stores import LiteLLM_ManagedVectorStore
from litellm.vector_stores import main as vector_stores
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

from openai_chat import Failure, project_failure as chat_failure

Operation = Literal["create", "search"]
Mode = Literal["sync", "async"]
Work = object | Coroutine[object, object, object]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

T = TypeVar("T")


class MissingArgumentsError(ValueError):
    def __init__(self, operation: str) -> None:
        super().__init__(f"a {operation} run carries no {operation} arguments")


class UnexpectedAnswerError(TypeError):
    def __init__(self, operation: str) -> None:
        super().__init__(f"vector store {operation} returned a coroutine for a sync call")


class ProjectedFailureError(TypeError):
    def __init__(self) -> None:
        super().__init__("the projected exception carries no failure")


class OptionalFields(BaseModel):
    model_config = ConfigDict(extra="forbid")

    name: str | None = None
    file_ids: list[str] | None = None
    expires_after: dict[str, JsonValue] | None = None
    chunking_strategy: dict[str, JsonValue] | None = None
    metadata: dict[str, str] | None = None


class SearchFields(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_store_id: str
    query: str | list[str]
    filters: dict[str, JsonValue] | None = None
    max_num_results: int | None = None
    ranking_options: dict[str, JsonValue] | None = None
    rewrite_query: bool | None = None


class RunOptions(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_key: str | None = None
    litellm_call_id: str
    mock_response: JsonValue = None


class Registry(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_store_id: str
    credential_name: str
    credential_values: dict[str, JsonValue] = Field(default_factory=dict)


class Run(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    mode: Mode
    create: OptionalFields | None = None
    search: SearchFields | None = None
    options: RunOptions
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, JsonValue] | None = None
    extra_query: dict[str, JsonValue] | None = None
    timeout_seconds: float | None = None
    custom_llm_provider: str | None = None


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[Run]
    registry: Registry | None = None


class Context(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    optional_params: dict[str, JsonValue]
    custom_llm_provider: str
    litellm_call_id: JsonValue
    vector_store_id: JsonValue


class RunOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    mode: Mode
    result: JsonValue = None
    failure: Failure | None = None
    context: Context | None = None


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[RunOutcome]


class Value(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: Outcome


class Envelope(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_store_search_create: Value


class SeenParams(TypedDict, total=False):
    litellm_call_id: JsonValue
    vector_store_id: JsonValue


class SeenDetails(TypedDict, total=False):
    model: str
    optional_params: dict[str, JsonValue]
    custom_llm_provider: str
    litellm_params: SeenParams


DETAILS: TypeAdapter[SeenDetails] = TypeAdapter(SeenDetails)
STORE: TypeAdapter[LiteLLM_ManagedVectorStore] = TypeAdapter(
    LiteLLM_ManagedVectorStore
)


class ContextRecorder(CustomLogger):
    def __init__(self) -> None:
        self.seen: Context | None = None

    @override
    def log_pre_api_call(
        self,
        model: str,
        messages: list[dict[str, str]],
        kwargs: dict[str, JsonValue],
    ) -> None:
        details = DETAILS.validate_python(kwargs)
        params = details["litellm_params"] if "litellm_params" in details else {}
        self.seen = Context(
            model=details["model"],
            optional_params=details["optional_params"],
            custom_llm_provider=details["custom_llm_provider"],
            litellm_call_id=params["litellm_call_id"]
            if "litellm_call_id" in params
            else None,
            vector_store_id=params["vector_store_id"]
            if "vector_store_id" in params
            else None,
        )


class GenericOptions(TypedDict, total=False):
    api_base: str | None
    api_key: str | None
    litellm_call_id: str
    litellm_logging_obj: Logging
    mock_response: JsonValue
    custom_llm_provider: str | None
    extra_headers: dict[str, str] | None
    extra_query: dict[str, JsonValue] | None
    extra_body: dict[str, JsonValue] | None
    timeout: float | None


class CreateOptions(GenericOptions, total=False):
    name: str | None
    file_ids: list[str] | None
    expires_after: dict[str, JsonValue] | None
    chunking_strategy: dict[str, JsonValue] | None
    metadata: dict[str, str] | None


class SearchOptions(GenericOptions, total=False):
    vector_store_id: str
    query: str | list[str]
    filters: dict[str, JsonValue] | None
    max_num_results: int | None
    ranking_options: dict[str, JsonValue] | None
    rewrite_query: bool | None


# both entries are unannotated **kwargs functions, so the calls go through the options they read
class VectorStores(Protocol):
    def create(self, **kwargs: Unpack[CreateOptions]) -> Work: ...

    async def acreate(self, **kwargs: Unpack[CreateOptions]) -> object: ...

    def search(self, **kwargs: Unpack[SearchOptions]) -> Work: ...

    async def asearch(self, **kwargs: Unpack[SearchOptions]) -> object: ...


FACADE: VectorStores = vector_stores


def logging_fixture(operation: Operation) -> Logging:
    return Logging(
        model="vector-store",
        messages=None,
        stream=False,
        call_type=f"vector_store_{operation}",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def required(held: T | None, operation: str) -> T:
    if held is None:
        raise MissingArgumentsError(operation)
    return held


def generic_options(
    run: Run, api_base: str, logging_obj: Logging
) -> GenericOptions:
    return GenericOptions(
        api_base=api_base,
        api_key=run.options.api_key,
        litellm_call_id=run.options.litellm_call_id,
        litellm_logging_obj=logging_obj,
        mock_response=run.options.mock_response,
        custom_llm_provider=run.custom_llm_provider,
        extra_headers=run.extra_headers,
        extra_query=run.extra_query,
        extra_body=run.extra_body,
        timeout=run.timeout_seconds,
    )


def create_options(run: Run, api_base: str, logging_obj: Logging) -> CreateOptions:
    args = required(run.create, "create")
    options = CreateOptions(**generic_options(run, api_base, logging_obj))
    options["name"] = args.name
    options["file_ids"] = args.file_ids
    options["expires_after"] = args.expires_after
    options["chunking_strategy"] = args.chunking_strategy
    options["metadata"] = args.metadata
    return options


def search_options(run: Run, api_base: str, logging_obj: Logging) -> SearchOptions:
    args = required(run.search, "search")
    options = SearchOptions(**generic_options(run, api_base, logging_obj))
    options["vector_store_id"] = args.vector_store_id
    options["query"] = args.query
    options["filters"] = args.filters
    options["max_num_results"] = args.max_num_results
    options["ranking_options"] = args.ranking_options
    options["rewrite_query"] = args.rewrite_query
    return options


def create_result(run: Run, api_base: str, logging_obj: Logging) -> JsonValue:
    options = create_options(run, api_base, logging_obj)
    if run.mode == "async":
        return JSON.validate_python(asyncio.run(FACADE.acreate(**options)))
    work = FACADE.create(**options)
    match work:
        case Coroutine():
            raise UnexpectedAnswerError("create")
        case object():
            return JSON.validate_python(work)


def search_result(run: Run, api_base: str, logging_obj: Logging) -> JsonValue:
    options = search_options(run, api_base, logging_obj)
    if run.mode == "async":
        return JSON.validate_python(asyncio.run(FACADE.asearch(**options)))
    work = FACADE.search(**options)
    match work:
        case Coroutine():
            raise UnexpectedAnswerError("search")
        case object():
            return JSON.validate_python(work)


def project_failure(error: openai.APIError) -> Failure:
    failure = chat_failure(error).failure
    if failure is None:
        raise ProjectedFailureError
    return failure


def run_once(run: Run, api_base: str) -> RunOutcome:
    logging_obj = logging_fixture(run.operation)
    recorder = ContextRecorder()
    logging_obj.add_dynamic_callback(recorder)
    try:
        if run.operation == "create":
            result = create_result(run, api_base, logging_obj)
        else:
            result = search_result(run, api_base, logging_obj)
    except openai.APIError as error:
        return RunOutcome(
            operation=run.operation,
            mode=run.mode,
            result=None,
            failure=project_failure(error),
            context=recorder.seen,
        )
    return RunOutcome(
        operation=run.operation,
        mode=run.mode,
        result=result,
        failure=None,
        context=recorder.seen,
    )


def apply_registry(registry: Registry) -> None:
    litellm.vector_store_registry = VectorStoreRegistry(
        vector_stores=[
            STORE.validate_python(
                {
                    "vector_store_id": registry.vector_store_id,
                    "litellm_credential_name": registry.credential_name,
                }
            )
        ]
    )
    litellm.credential_list = [
        CredentialItem(
            credential_name=registry.credential_name,
            credential_info={},
            credential_values=registry.credential_values,
        )
    ]


def run_call(call: Call, api_base: str) -> Envelope:
    held_registry = litellm.vector_store_registry
    held_credentials = litellm.credential_list
    try:
        if call.registry is not None:
            apply_registry(call.registry)
        runs = [run_once(run, api_base) for run in call.runs]
    finally:
        litellm.vector_store_registry = held_registry
        litellm.credential_list = held_credentials
    return Envelope(vector_store_search_create=Value(value=Outcome(runs=runs)))


class VectorStoreSearchCreateRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_store_search_create: Call

    def serialize(self, api_base: str) -> str:
        return run_call(self.vector_store_search_create, api_base).model_dump_json(
            by_alias=True, exclude_unset=True
        )
