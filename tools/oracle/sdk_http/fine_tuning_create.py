import asyncio
from collections.abc import Awaitable, Coroutine
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
import openai
from litellm import acreate_fine_tuning_job, create_fine_tuning_job
from litellm.fine_tuning import main as fine_tuning_main
from litellm.integrations.gcs_bucket import gcs_bucket_mock_client
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.utils import LiteLLMFineTuningJob
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter
from pydantic import JsonValue as Json
from typing_extensions import override

from openai_chat import Failure, project_failure

# LiteLLM's sync create returns the job, or a coroutine of it on its internal async path
CreateAnswer: TypeAlias = LiteLLMFineTuningJob | Awaitable[LiteLLMFineTuningJob]


class RunParams(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_base: str | None = None
    api_key: str | None = None
    api_version: str | None = None
    organization: str | None = None
    vertex_project: str | None = None
    vertex_location: str | None = None
    vertex_credentials: str | None = None
    training_type: int | float | str | bool | None = Field(default=None, alias="trainingType")
    timeout: float | str | None = None
    request_timeout: float | None = None
    max_retries: int | None = None


class Run(BaseModel):
    model_config = ConfigDict(extra="forbid")

    provider: str
    is_async: bool = False
    model: str
    training_file: str
    hyperparameters: dict[str, Json] | None = None
    suffix: str | None = None
    validation_file: str | None = None
    integrations: list[str] | None = None
    seed: int | None = None
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, Json] | None = None
    params: RunParams = Field(default_factory=RunParams)


class JobView(TypedDict):
    id: str
    object: str
    created_at: int
    model: str
    status: str
    training_file: str
    validation_file: str | None
    organization_id: str
    result_files: list[str]
    fine_tuned_model: str | None
    seed: int | None
    hyperparameters: dict[str, Json]


class RunOutcome(TypedDict):
    job: JobView | None
    failure: Failure | None
    request_url: str | None


class Outcome(TypedDict):
    fine_tuning_create: dict[Literal["value"], dict[Literal["runs"], list[RunOutcome]]]


class CreateOptions(TypedDict):
    model: str
    training_file: str
    hyperparameters: dict[str, Json] | None
    suffix: str | None
    validation_file: str | None
    integrations: list[str] | None
    seed: int | None
    custom_llm_provider: str
    extra_headers: dict[str, str] | None
    extra_body: dict[str, Json] | None
    api_base: str
    api_key: str | None
    api_version: str | None
    organization: str | None
    vertex_project: str | None
    vertex_location: str | None
    vertex_credentials: str | None
    timeout: float | str | None
    request_timeout: float | None
    max_retries: int | None
    trainingType: NotRequired[int | float | str | bool]


class FineTuningCreateCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[Run]


class FineTuningCreateRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    fine_tuning_create: FineTuningCreateCall

    def serialize(self, api_base: str) -> str:
        return run_fine_tuning_create(self.fine_tuning_create, api_base)


class VertexAuth(Protocol):
    def mock_vertex_auth_methods(self) -> None: ...


class VertexRecorder(httpx.AsyncBaseTransport):
    def __init__(self, origin: httpx.URL, urls: list[str]) -> None:
        self.origin = origin
        self.urls = urls
        self.inner = httpx.AsyncHTTPTransport()

    @override
    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        self.urls.append(str(request.url))
        forwarded = httpx.Request(
            method=request.method,
            url=request.url.copy_with(
                scheme=self.origin.scheme, host=self.origin.host, port=self.origin.port
            ),
            headers=request.headers,
            content=request.content,
        )
        return await self.inner.handle_async_request(forwarded)


def request_options(run: Run, api_base: str) -> dict[str, Json]:
    options: CreateOptions = {
        "model": run.model,
        "training_file": run.training_file,
        "hyperparameters": run.hyperparameters,
        "suffix": run.suffix,
        "validation_file": run.validation_file,
        "integrations": run.integrations,
        "seed": run.seed,
        "custom_llm_provider": run.provider,
        "extra_headers": run.extra_headers,
        "extra_body": run.extra_body,
        "api_base": api_base,
        "api_key": run.params.api_key,
        "api_version": run.params.api_version,
        "organization": run.params.organization,
        "vertex_project": run.params.vertex_project,
        "vertex_location": run.params.vertex_location,
        "vertex_credentials": run.params.vertex_credentials,
        "timeout": run.params.timeout,
        "request_timeout": run.params.request_timeout,
        "max_retries": run.params.max_retries,
    }
    if run.params.training_type is not None:
        options["trainingType"] = run.params.training_type
    return TypeAdapter(dict[str, Json]).validate_python(options)


def invoke(run: Run, api_base: str) -> LiteLLMFineTuningJob:
    options = request_options(run, api_base)
    if run.is_async:
        held: Coroutine[None, None, LiteLLMFineTuningJob] = acreate_fine_tuning_job(
            **options
        )
        return asyncio.run(held)
    answer: CreateAnswer = create_fine_tuning_job(**options)
    if asyncio.iscoroutine(answer):
        pending: Coroutine[None, None, LiteLLMFineTuningJob] = answer
        return asyncio.run(pending)
    return TypeAdapter(LiteLLMFineTuningJob).validate_python(answer)


def run_one(run: Run, api_base: str) -> RunOutcome:
    urls: list[str] = []
    previous = fine_tuning_main.vertex_fine_tuning_apis_instance.async_handler
    if run.provider == "vertex_ai":
        fine_tuning_main.vertex_fine_tuning_apis_instance.async_handler = AsyncHTTPHandler(
            transport=VertexRecorder(httpx.URL(api_base), urls)
        )
    try:
        response = invoke(run, api_base)
    except openai.APIError as error:
        job = None
        failure = project_failure(error).failure
    else:
        job = TypeAdapter(JobView).validate_json(response.model_dump_json())
        failure = None
    finally:
        fine_tuning_main.vertex_fine_tuning_apis_instance.async_handler = previous
    return {"job": job, "failure": failure, "request_url": urls[0] if urls else None}


def run_fine_tuning_create(call: FineTuningCreateCall, api_base: str) -> str:
    vertex_auth: VertexAuth = gcs_bucket_mock_client
    vertex_auth.mock_vertex_auth_methods()
    runs = [run_one(run, api_base) for run in call.runs]
    outcome: Outcome = {"fine_tuning_create": {"value": {"runs": runs}}}
    return TypeAdapter(Outcome).dump_json(outcome, by_alias=True, exclude_unset=True).decode()
