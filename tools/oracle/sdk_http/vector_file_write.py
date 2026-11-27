import asyncio
from collections.abc import Awaitable, Mapping
from typing import Literal, Protocol

import litellm
import litellm.vector_store_files.main as vector_file_main
import openai
from pydantic import BaseModel, ConfigDict, Field, JsonValue
from typing_extensions import override

from openai_chat import Failure, project_failure

Operation = Literal["update", "delete"]
Mode = Literal["sync", "async"]


# Both entries are wrapped by @client, which leaves their signatures untyped, and their keyword-only arguments do not fit a named parameter here
class WriteFacade(Protocol):
    def update(self, **options: JsonValue) -> JsonValue: ...

    def aupdate(self, **options: JsonValue) -> Awaitable[JsonValue]: ...

    def delete(self, **options: JsonValue) -> JsonValue: ...

    def adelete(self, **options: JsonValue) -> Awaitable[JsonValue]: ...


WRITES: WriteFacade = vector_file_main  # pyright: ignore[reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py#L523-L749


class Registry(BaseModel):
    model_config = ConfigDict(extra="forbid")

    credentials: dict[str, JsonValue] = Field(default_factory=dict)
    failing: bool = False


class Run(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    mode: Mode
    vector_store_id: str
    file_id: str
    attributes: JsonValue = None
    extra_headers: JsonValue = None
    extra_body: JsonValue = None
    extra_query: JsonValue = None
    unknown: JsonValue = None
    api_key: str | None = None
    timeout: float | None = None
    custom_llm_provider: str | None = None
    registry: Registry | None = None


class VectorFileWriteCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[Run]


class RunOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: JsonValue = None
    failure: Failure | None = None


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[RunOutcome]


class VectorFileWriteValue(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: Outcome


class VectorFileWriteResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_file_write: VectorFileWriteValue


# The case's registry: the credentials it overlays, or a lookup that raises and LiteLLM swallows
class CaseRegistry(litellm.VectorStoreRegistry):
    credentials: Mapping[str, object]  # ast-grep-ignore: no-loose-types
    failing: bool

    def __init__(self, registry: Registry) -> None:
        super().__init__()
        self.credentials = registry.credentials
        self.failing = registry.failing

    @override
    def get_credentials_for_vector_store(  # ast-grep-ignore: no-long-names
        self, vector_store_id: str
    ) -> dict[str, object]:  # ast-grep-ignore: no-loose-types
        if self.failing:
            raise RuntimeError(f"the registry has no credentials for {vector_store_id}")
        return dict(self.credentials)


# Every keyword the case sends: update always gets its attributes, even a null, and a delete's extra_body rides in **kwargs
def write_options(run: Run, api_base: str) -> dict[str, JsonValue]:
    options: dict[str, JsonValue] = {
        "vector_store_id": run.vector_store_id,
        "file_id": run.file_id,
        "extra_headers": run.extra_headers,
        "extra_body": run.extra_body,
        "timeout": run.timeout,
        "custom_llm_provider": run.custom_llm_provider,
        "api_base": api_base,
    }
    if run.operation == "update":
        options["attributes"] = run.attributes
    if run.api_key is not None:
        options["api_key"] = run.api_key
    if run.extra_query is not None:
        options["extra_query"] = run.extra_query
    if run.unknown is not None:
        options["unknown"] = run.unknown
    return options


def update_write(run: Run, api_base: str) -> JsonValue:
    return WRITES.update(**write_options(run, api_base))


def delete_write(run: Run, api_base: str) -> JsonValue:
    return WRITES.delete(**write_options(run, api_base))


async def aupdate_write(run: Run, api_base: str) -> JsonValue:
    return await WRITES.aupdate(**write_options(run, api_base))


async def adelete_write(run: Run, api_base: str) -> JsonValue:
    return await WRITES.adelete(**write_options(run, api_base))


def written(run: Run, api_base: str) -> JsonValue:
    if run.operation == "update":
        if run.mode == "sync":
            return update_write(run, api_base)
        return asyncio.run(aupdate_write(run, api_base))
    if run.mode == "sync":
        return delete_write(run, api_base)
    return asyncio.run(adelete_write(run, api_base))


def run_once(run: Run, api_base: str) -> RunOutcome:
    previous = litellm.vector_store_registry
    litellm.vector_store_registry = None if run.registry is None else CaseRegistry(run.registry)
    try:
        value = written(run, api_base)
    except openai.APIError as error:
        return RunOutcome(failure=project_failure(error).failure)
    finally:
        litellm.vector_store_registry = previous
    return RunOutcome(value=value)


class VectorFileWriteRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_file_write: VectorFileWriteCall

    def serialize(self, api_base: str) -> str:
        runs = [run_once(run, api_base) for run in self.vector_file_write.runs]
        value = VectorFileWriteValue(value=Outcome(runs=runs))
        return VectorFileWriteResult(vector_file_write=value).model_dump_json(by_alias=True)
