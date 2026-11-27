import asyncio
import json
from collections.abc import Coroutine
from datetime import UTC, datetime
from functools import partial
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

import litellm
import openai
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.vector_store_files import main as vector_store_files
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

from openai_chat import Failure
from openai_chat import project_failure as chat_failure

LITELLM = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm"

Operation = Literal["retrieve", "content"]
Mode = Literal["sync", "async"]
Answer: TypeAlias = JsonValue | Coroutine[None, None, JsonValue]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class RunOptions(Strict):
    api_key: str | None = None
    litellm_call_id: str | None = None
    extra_query: dict[str, JsonValue] | None = None
    after: str | None = None
    limit: int | None = None
    max_retries: int | None = None


class Registry(Strict):
    vector_store_id: str
    credential_name: str
    credential_values: dict[str, JsonValue] = Field(default_factory=dict)


class Run(Strict):
    operation: Operation
    mode: Mode
    vector_store_id: str
    file_id: str
    options: RunOptions = Field(default_factory=RunOptions)
    extra_headers: dict[str, str] | None = None
    timeout_seconds: int | None = None
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


class ReadOptions(TypedDict, total=False):
    api_base: str | None
    api_key: str | None
    litellm_call_id: str | None
    litellm_logging_obj: Logging
    custom_llm_provider: str | None
    extra_headers: dict[str, str] | None
    extra_query: dict[str, JsonValue] | None
    after: str | None
    limit: int | None
    max_retries: int | None
    timeout: int | None


# the four entries take keyword-only **kwargs, so the calls go through the options they read
class VectorStoreFiles(Protocol):
    def retrieve(self, **kwargs: Unpack[ReadOptions]) -> Answer: ...

    async def aretrieve(self, **kwargs: Unpack[ReadOptions]) -> JsonValue: ...

    def retrieve_content(self, **kwargs: Unpack[ReadOptions]) -> Answer: ...

    async def aretrieve_content(self, **kwargs: Unpack[ReadOptions]) -> JsonValue: ...


FACADE: VectorStoreFiles = cast(VectorStoreFiles, vector_store_files)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py


def optionals(run: Run, api_base: str, logging_obj: Logging) -> ReadOptions:
    return ReadOptions(
        api_base=api_base,
        api_key=run.options.api_key,
        litellm_call_id=run.options.litellm_call_id,
        litellm_logging_obj=logging_obj,
        custom_llm_provider=run.custom_llm_provider,
        extra_headers=run.extra_headers,
        extra_query=run.options.extra_query,
        after=run.options.after,
        limit=run.options.limit,
        max_retries=run.options.max_retries,
        timeout=run.timeout_seconds,
    )


def answer(run: Run, api_base: str, logging_obj: Logging) -> JsonValue:
    options = optionals(run, api_base, logging_obj)
    if run.operation == "retrieve":
        if run.mode == "async":
            return JSON.validate_python(asyncio.run(FACADE.aretrieve(**options)))
        return JSON.validate_python(FACADE.retrieve(**options))
    if run.mode == "async":
        return JSON.validate_python(asyncio.run(FACADE.aretrieve_content(**options)))
    return JSON.validate_python(FACADE.retrieve_content(**options))


def logging_fixture(run: Run) -> Logging:
    return Logging(
        model="vector-store",
        messages=None,
        stream=False,
        call_type=f"vector_store_file_{run.operation}",
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


class VectorFileReadRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_file_read: Call

    def serialize(self, api_base: str) -> str:
        held = litellm.vector_store_registry
        try:
            if self.vector_file_read.registry is not None:
                apply_registry(self.vector_file_read.registry)
            runs = [run_once(run, api_base) for run in self.vector_file_read.runs]
        finally:
            litellm.vector_store_registry = held
        payload: dict[str, JsonValue] = {
            "vector_file_read": {
                "value": {
                    "runs": [
                        JSON.validate_json(run.model_dump_json(by_alias=True))
                        for run in runs
                    ]
                }
            }
        }
        return json.dumps(payload, separators=(",", ":"))
