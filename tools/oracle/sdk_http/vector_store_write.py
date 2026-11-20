import asyncio
from collections.abc import Coroutine
from typing import Literal, Protocol, TypedDict, Unpack

import litellm.vector_stores.main as vector_store_main
import openai
from litellm.types.vector_stores import VectorStoreCreateResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

from openai_chat import Failure, project_failure

Operation = Literal["update", "delete"]
Mode = Literal["sync", "async"]

AsyncUpdateWork = Coroutine[object, object, VectorStoreCreateResponse]
AsyncDeleteWork = Coroutine[object, object, JsonValue]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class WriteOptions(TypedDict, total=False):
    api_base: str
    name: str
    expires_after: JsonValue
    metadata: JsonValue
    file_ids: list[str]
    chunking_strategy: JsonValue
    unknown: JsonValue
    extra_headers: JsonValue
    extra_query: JsonValue
    extra_body: JsonValue
    timeout: float
    custom_llm_provider: str


# The four entries take optional arguments as keywords, so one options map feeds both modes
class WriteFacade(Protocol):
    def update(
        self, vector_store_id: str, **options: Unpack[WriteOptions]
    ) -> VectorStoreCreateResponse: ...

    def aupdate(
        self, vector_store_id: str, **options: Unpack[WriteOptions]
    ) -> AsyncUpdateWork: ...

    def delete(self, vector_store_id: str, **options: Unpack[WriteOptions]) -> JsonValue: ...

    def adelete(
        self, vector_store_id: str, **options: Unpack[WriteOptions]
    ) -> AsyncDeleteWork: ...


WRITES: WriteFacade = vector_store_main


class Run(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    mode: Mode
    vector_store_id: str
    custom_llm_provider: str | None = None
    name: str | None = None
    expires_after: JsonValue = None
    metadata: JsonValue = None
    file_ids: list[str] | None = None
    chunking_strategy: JsonValue = None
    unknown: JsonValue = None
    extra_headers: JsonValue = None
    extra_query: JsonValue = None
    extra_body: JsonValue = None
    timeout: float | None = None


class VectorStoreWriteCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[Run]


class RunOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: JsonValue = None
    failure: Failure | None = None


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[RunOutcome]


class VectorStoreWriteValue(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: Outcome


class VectorStoreWriteResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_store_write: VectorStoreWriteValue


class VectorStoreWriteRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_store_write: VectorStoreWriteCall

    def serialize(self, api_base: str) -> str:
        runs = [run_once(run, api_base) for run in self.vector_store_write.runs]
        value = VectorStoreWriteValue(value=Outcome(runs=runs))
        return VectorStoreWriteResult(vector_store_write=value).model_dump_json(by_alias=True)


def transport_options(run: Run, api_base: str) -> WriteOptions:
    options: WriteOptions = {"api_base": api_base}
    if run.extra_headers is not None:
        options["extra_headers"] = run.extra_headers
    if run.extra_query is not None:
        options["extra_query"] = run.extra_query
    if run.extra_body is not None:
        options["extra_body"] = run.extra_body
    if run.timeout is not None:
        options["timeout"] = run.timeout
    if run.custom_llm_provider is not None:
        options["custom_llm_provider"] = run.custom_llm_provider
    return options


# update reads file_ids and chunking_strategy from **kwargs; the create filter keeps only those
def update_options(run: Run, api_base: str) -> WriteOptions:
    options = transport_options(run, api_base)
    if run.name is not None:
        options["name"] = run.name
    if run.expires_after is not None:
        options["expires_after"] = run.expires_after
    if run.metadata is not None:
        options["metadata"] = run.metadata
    if run.file_ids is not None:
        options["file_ids"] = run.file_ids
    if run.chunking_strategy is not None:
        options["chunking_strategy"] = run.chunking_strategy
    if run.unknown is not None:
        options["unknown"] = run.unknown
    return options


def update_write(run: Run, api_base: str) -> JsonValue:
    answer = WRITES.update(run.vector_store_id, **update_options(run, api_base))
    return JSON.validate_python(answer)


def aupdate_write(run: Run, api_base: str) -> JsonValue:
    held = WRITES.aupdate(run.vector_store_id, **update_options(run, api_base))
    return JSON.validate_python(asyncio.run(held))


def delete_write(run: Run, api_base: str) -> JsonValue:
    answer = WRITES.delete(run.vector_store_id, **transport_options(run, api_base))
    return JSON.validate_python(answer)


def adelete_write(run: Run, api_base: str) -> JsonValue:
    held = WRITES.adelete(run.vector_store_id, **transport_options(run, api_base))
    return JSON.validate_python(asyncio.run(held))


def written(run: Run, api_base: str) -> JsonValue:
    if run.operation == "update":
        return update_write(run, api_base) if run.mode == "sync" else aupdate_write(run, api_base)
    return delete_write(run, api_base) if run.mode == "sync" else adelete_write(run, api_base)


def run_once(run: Run, api_base: str) -> RunOutcome:
    try:
        value = written(run, api_base)
    except openai.APIError as error:
        return RunOutcome(failure=project_failure(error).failure)
    return RunOutcome(value=value)
