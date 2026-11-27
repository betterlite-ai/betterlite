import asyncio
import json
from collections.abc import Coroutine
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, Protocol, TypeAlias, cast

import litellm
import litellm.vector_store_files.main as vector_file_main
import openai
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

from openai_chat import Failure
from openai_chat import project_failure as chat_failure

Operation = Literal["create", "list"]
Mode = Literal["sync", "async"]
Answer: TypeAlias = JsonValue | Coroutine[None, None, JsonValue]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
API_BASE_SENTINEL: Final = "$API_BASE"


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class CreateFields(Strict):
    file_id: str
    attributes: dict[str, str | int | float | bool] | None = None
    chunking_strategy: dict[str, JsonValue] | None = None


class ListFields(Strict):
    after: str | None = None
    before: str | None = None
    filter_status: str | None = Field(default=None, alias="filter")
    limit: int | None = None
    order: str | None = None


class RunOptions(Strict):
    api_key: str | None = None
    litellm_call_id: str


class Registry(Strict):
    vector_store_id: str
    credential_values: dict[str, JsonValue] = Field(default_factory=dict)
    failing: bool = False


class Run(Strict):
    operation: Operation
    mode: Mode
    vector_store_id: str
    create: CreateFields | None = None
    list: ListFields | None = None
    options: RunOptions
    extra_headers: dict[str, str] | None = None
    extra_query: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    timeout_seconds: float | None = None
    custom_llm_provider: str | None = None
    unknown: JsonValue = None


class Call(Strict):
    runs: list[Run]
    registry: Registry | None = None


class RunOutcome(Strict):
    operation: Operation
    mode: Mode
    result: JsonValue = None
    failure: Failure | None = None


class VectorFiles(Protocol):
    def create(self, **options: JsonValue | Logging) -> Answer: ...

    async def acreate(self, **options: JsonValue | Logging) -> JsonValue: ...

    def list(self, **options: JsonValue | Logging) -> Answer: ...

    async def alist(self, **options: JsonValue | Logging) -> JsonValue: ...


FILES: VectorFiles = cast(VectorFiles, vector_file_main)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_store_files/main.py


def optionals(
    run: Run, api_base: str, logging_obj: Logging
) -> dict[str, JsonValue | Logging]:
    return {
        "api_base": api_base,
        "api_key": run.options.api_key,
        "litellm_call_id": run.options.litellm_call_id,
        "litellm_logging_obj": logging_obj,
        "extra_headers": (
            None
            if run.extra_headers is None
            else dict[str, JsonValue](run.extra_headers)
        ),
        "extra_query": run.extra_query,
        "timeout": run.timeout_seconds,
        "custom_llm_provider": run.custom_llm_provider,
        "unknown": run.unknown,
    }


def create_options(
    run: Run, api_base: str, logging_obj: Logging
) -> dict[str, JsonValue | Logging]:
    args = run.create
    if args is None:
        raise ValueError("a create run carries no create arguments")
    options = optionals(run, api_base, logging_obj)
    options["vector_store_id"] = run.vector_store_id
    options["file_id"] = args.file_id
    options["attributes"] = (
        None
        if args.attributes is None
        else dict[str, JsonValue](args.attributes)
    )
    options["chunking_strategy"] = args.chunking_strategy
    options["extra_body"] = run.extra_body
    return options


def list_options(
    run: Run, api_base: str, logging_obj: Logging
) -> dict[str, JsonValue | Logging]:
    args = run.list
    if args is None:
        raise ValueError("a list run carries no list arguments")
    options = optionals(run, api_base, logging_obj)
    options["vector_store_id"] = run.vector_store_id
    options["after"] = args.after
    options["before"] = args.before
    options["filter"] = args.filter_status
    options["limit"] = args.limit
    options["order"] = args.order
    return options


def answer(run: Run, api_base: str, logging_obj: Logging) -> JsonValue:
    if run.operation == "create":
        options = create_options(run, api_base, logging_obj)
        if run.mode == "async":
            return JSON.validate_python(asyncio.run(FILES.acreate(**options)))
        return JSON.validate_python(FILES.create(**options))
    listing = list_options(run, api_base, logging_obj)
    if run.mode == "async":
        return JSON.validate_python(asyncio.run(FILES.alist(**listing)))
    return JSON.validate_python(FILES.list(**listing))


def logging_fixture(run: Run) -> Logging:
    return Logging(
        model="",
        messages=None,
        stream=False,
        call_type=f"vector_store_file_{run.operation}",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def run_once(run: Run, api_base: str) -> RunOutcome:
    logging_obj = logging_fixture(run)
    try:
        result = answer(run, api_base, logging_obj)
    except openai.APIError as error:
        failure = chat_failure(error).failure
        if failure is None:
            raise ValueError("the projected exception carries no failure") from error
        return RunOutcome(operation=run.operation, mode=run.mode, failure=failure)
    return RunOutcome(operation=run.operation, mode=run.mode, result=result)


def credentials_for(
    case: Registry, api_base: str, vector_store_id: str
) -> dict[str, JsonValue]:
    if case.failing:
        raise RuntimeError("the case registry raised")
    if vector_store_id != case.vector_store_id:
        return {}
    return {
        name: api_base if value == API_BASE_SENTINEL else value
        for name, value in case.credential_values.items()
    }


def apply_registry(case: Registry, api_base: str) -> None:
    registry = VectorStoreRegistry()
    registry.get_credentials_for_vector_store = partial(credentials_for, case, api_base)  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py
    litellm.vector_store_registry = registry


class VectorFileCreateListRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    vector_file_create_list: Call

    def serialize(self, api_base: str) -> str:
        held = litellm.vector_store_registry
        try:
            if self.vector_file_create_list.registry is not None:
                apply_registry(self.vector_file_create_list.registry, api_base)
            runs = [run_once(run, api_base) for run in self.vector_file_create_list.runs]
        finally:
            litellm.vector_store_registry = held
        payload: dict[str, JsonValue] = {
            "vector_file_create_list": {
                "value": {
                    "runs": [
                        JSON.validate_json(run.model_dump_json(by_alias=True))
                        for run in runs
                    ]
                }
            }
        }
        return json.dumps(payload, separators=(",", ":"))
