import asyncio
import json
from collections.abc import Coroutine
from datetime import UTC, datetime
from functools import partial
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

import litellm
import openai
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.vector_stores import main as vector_stores
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

from openai_chat import Failure
from openai_chat import project_failure as chat_failure

LITELLM = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm"

Operation = Literal["create", "search"]
Mode = Literal["sync", "async"]
Answer: TypeAlias = JsonValue | Coroutine[None, None, JsonValue]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class CreateFields(Strict):
    name: str | None = None
    file_ids: list[str] | None = None
    expires_after: dict[str, JsonValue] | None = None
    chunking_strategy: dict[str, JsonValue] | None = None
    metadata: dict[str, str] | None = None


class SearchFields(Strict):
    vector_store_id: str
    query: str | list[str]
    filters: dict[str, JsonValue] | None = None
    max_num_results: int | None = None
    ranking_options: dict[str, JsonValue] | None = None
    rewrite_query: bool | None = None


class RunOptions(Strict):
    api_key: str | None = None
    litellm_call_id: str
    mock_response: JsonValue = None


class Registry(Strict):
    vector_store_id: str
    credential_name: str
    credential_values: dict[str, JsonValue] = Field(default_factory=dict)


class Run(Strict):
    operation: Operation
    mode: Mode
    create: CreateFields | None = None
    search: SearchFields | None = None
    options: RunOptions
    extra_headers: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    extra_query: dict[str, JsonValue] | None = None
    timeout_seconds: float | None = None
    custom_llm_provider: str | None = None


class Call(Strict):
    runs: list[Run]
    registry: Registry | None = None


class Context(Strict):
    model: str
    optional_params: dict[str, JsonValue]
    custom_llm_provider: str
    litellm_call_id: JsonValue
    vector_store_id: JsonValue


class RunOutcome(Strict):
    operation: Operation
    mode: Mode
    result: JsonValue = None
    failure: Failure | None = None
    context: Context | None = None


class SeenParams(TypedDict, total=False):
    litellm_call_id: JsonValue
    vector_store_id: JsonValue


class SeenDetails(TypedDict):
    model: str
    optional_params: dict[str, JsonValue]
    custom_llm_provider: NotRequired[str]
    litellm_params: SeenParams


class GenericOptions(TypedDict, total=False):
    api_base: str | None
    api_key: str | None
    litellm_call_id: str
    litellm_logging_obj: Logging
    mock_response: JsonValue
    custom_llm_provider: str | None
    extra_headers: dict[str, JsonValue] | None
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
    def create(self, **kwargs: Unpack[CreateOptions]) -> Answer: ...

    async def acreate(self, **kwargs: Unpack[CreateOptions]) -> JsonValue: ...

    def search(self, **kwargs: Unpack[SearchOptions]) -> Answer: ...

    async def asearch(self, **kwargs: Unpack[SearchOptions]) -> JsonValue: ...


FACADE: VectorStores = cast(VectorStores, vector_stores)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py


def optionals(run: Run, api_base: str, logging_obj: Logging) -> CreateOptions:
    return CreateOptions(
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
    args = run.create
    if args is None:
        raise ValueError("a create run carries no create arguments")
    options = optionals(run, api_base, logging_obj)
    options["name"] = args.name
    options["file_ids"] = args.file_ids
    options["expires_after"] = args.expires_after
    options["chunking_strategy"] = args.chunking_strategy
    options["metadata"] = args.metadata
    return options


def search_options(run: Run, api_base: str, logging_obj: Logging) -> SearchOptions:
    args = run.search
    if args is None:
        raise ValueError("a search run carries no search arguments")
    return {
        "api_base": api_base,
        "api_key": run.options.api_key,
        "litellm_call_id": run.options.litellm_call_id,
        "litellm_logging_obj": logging_obj,
        "mock_response": run.options.mock_response,
        "custom_llm_provider": run.custom_llm_provider,
        "extra_headers": run.extra_headers,
        "extra_query": run.extra_query,
        "extra_body": run.extra_body,
        "timeout": run.timeout_seconds,
        "vector_store_id": args.vector_store_id,
        "query": args.query,
        "filters": args.filters,
        "max_num_results": args.max_num_results,
        "ranking_options": args.ranking_options,
        "rewrite_query": args.rewrite_query,
    }


def answer(run: Run, api_base: str, logging_obj: Logging) -> JsonValue:
    if run.operation == "create":
        options = create_options(run, api_base, logging_obj)
        if run.mode == "async":
            return JSON.validate_python(asyncio.run(FACADE.acreate(**options)))
        return JSON.validate_python(FACADE.create(**options))
    search = search_options(run, api_base, logging_obj)
    if run.mode == "async":
        return JSON.validate_python(asyncio.run(FACADE.asearch(**search)))
    return JSON.validate_python(FACADE.search(**search))


def logging_fixture(run: Run) -> Logging:
    return Logging(
        model="vector-store",
        messages=None,
        stream=False,
        call_type=f"vector_store_{run.operation}",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def context_of(logging_obj: Logging) -> Context | None:
    details = cast(SeenDetails, logging_obj.model_call_details)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py
    provider = details.get("custom_llm_provider")
    if provider is None:
        return None
    params = details["litellm_params"]
    return Context(
        model=details["model"],
        optional_params=details["optional_params"],
        custom_llm_provider=provider,
        litellm_call_id=params.get("litellm_call_id"),
        vector_store_id=params.get("vector_store_id"),
    )


def run_once(run: Run, api_base: str) -> RunOutcome:
    logging_obj = logging_fixture(run)
    try:
        result = answer(run, api_base, logging_obj)
    except openai.APIError as error:
        failure = chat_failure(error).failure
        if failure is None:
            raise ValueError("the projected exception carries no failure") from error
        return RunOutcome(
            operation=run.operation,
            mode=run.mode,
            result=None,
            failure=failure,
            context=context_of(logging_obj),
        )
    return RunOutcome(
        operation=run.operation,
        mode=run.mode,
        result=result,
        failure=None,
        context=context_of(logging_obj),
    )


def credentials_for(case: Registry, vector_store_id: str) -> dict[str, JsonValue]:
    if vector_store_id == case.vector_store_id:
        return dict(case.credential_values)
    return {}


def apply_registry(case: Registry) -> None:
    registry = VectorStoreRegistry()
    registry.get_credentials_for_vector_store = partial(credentials_for, case)  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py
    litellm.vector_store_registry = registry


class VectorStoreSearchCreateRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_store_search_create: Call

    def serialize(self, api_base: str) -> str:
        held = litellm.vector_store_registry
        try:
            if self.vector_store_search_create.registry is not None:
                apply_registry(self.vector_store_search_create.registry)
            runs = [run_once(run, api_base) for run in self.vector_store_search_create.runs]
        finally:
            litellm.vector_store_registry = held
        payload: dict[str, JsonValue] = {
            "vector_store_search_create": {
                "value": {"runs": [JSON.validate_json(run.model_dump_json()) for run in runs]}
            }
        }
        return json.dumps(payload, separators=(",", ":"))
