import asyncio
from collections.abc import Coroutine, Mapping
from typing import Literal, Protocol

import litellm.vector_stores.main as vector_store_main
import openai
from litellm.types.vector_stores import VectorStoreCreateResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

from openai_chat import Failure, project_failure

Operation = Literal["update", "delete"]
Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class WriteFacade(Protocol):
    def update(
        self,
        vector_store_id: str,
        **options: JsonValue,
    ) -> VectorStoreCreateResponse | Coroutine[object, object, VectorStoreCreateResponse]: ...

    def aupdate(
        self,
        vector_store_id: str,
        **options: JsonValue,
    ) -> Coroutine[object, object, VectorStoreCreateResponse]: ...

    def delete(self, vector_store_id: str, **options: JsonValue) -> JsonValue: ...

    def adelete(
        self,
        vector_store_id: str,
        **options: JsonValue,
    ) -> Coroutine[object, object, Mapping[str, object]]: ...


# LiteLLM annotates metadata and the extra fields more narrowly than the JSON it forwards
WRITES: WriteFacade = vector_store_main  # pyright: ignore[reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/main.py#L841-L846


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


# update reads file_ids and chunking_strategy from **kwargs; null takes the SDK default
def write_options(run: Run, api_base: str) -> dict[str, JsonValue]:
    return {
        "api_base": api_base,
        "name": run.name,
        "expires_after": run.expires_after,
        "metadata": run.metadata,
        "file_ids": JSON.validate_python(run.file_ids),
        "chunking_strategy": run.chunking_strategy,
        "unknown": run.unknown,
        "extra_headers": run.extra_headers,
        "extra_query": run.extra_query,
        "extra_body": run.extra_body,
        "timeout": run.timeout,
        "custom_llm_provider": run.custom_llm_provider,
    }


def update_write(run: Run, api_base: str) -> JsonValue:
    answer = WRITES.update(run.vector_store_id, **write_options(run, api_base))
    return JSON.validate_python(answer)


def aupdate_write(run: Run, api_base: str) -> JsonValue:
    held = WRITES.aupdate(run.vector_store_id, **write_options(run, api_base))
    return JSON.validate_python(asyncio.run(held))


def delete_write(run: Run, api_base: str) -> JsonValue:
    answer = WRITES.delete(run.vector_store_id, **write_options(run, api_base))
    return JSON.validate_python(answer)


def adelete_write(run: Run, api_base: str) -> JsonValue:
    held = WRITES.adelete(run.vector_store_id, **write_options(run, api_base))
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
