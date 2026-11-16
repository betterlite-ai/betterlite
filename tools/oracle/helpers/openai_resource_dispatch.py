import asyncio
from collections.abc import Callable
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.llms.openai.fine_tuning.handler import OpenAIFineTuningAPI
from litellm.llms.openai.openai import OpenAIBatchesAPI, OpenAIFilesAPI
from litellm.llms.openai.vector_stores.transformation import OpenAIVectorStoreConfig
from litellm.types.llms.openai import CreateBatchRequest, CreateFileRequest
from litellm.types.router import GenericLiteLLMParams
from litellm.types.vector_stores import VectorStoreCreateOptionalRequestParams
from openai import AsyncOpenAI
from pydantic import JsonValue, TypeAdapter

from openai_containers import ContainerConfig, container_config
from openai_evals.config import CONFIG as EVALS

JSON: Final = TypeAdapter(JsonValue)


class VectorConfig(Protocol):
    # ast-grep-ignore: no-long-names
    def transform_create_vector_store_request(
        self, params: VectorStoreCreateOptionalRequestParams, api_base: str
    ) -> tuple[str, dict[str, JsonValue]]: ...


VECTOR: Final[VectorConfig] = OpenAIVectorStoreConfig()


class FilesCase(TypedDict):
    case: Literal["files_create"]
    api_base: str
    status: int
    filename: str
    content: str
    reply: dict[str, JsonValue]


class BatchesCase(TypedDict):
    case: Literal["batches_create"]
    api_base: str
    status: int
    create: CreateBatchRequest
    reply: dict[str, JsonValue]


class FineTuningCase(TypedDict):
    case: Literal["fine_tuning_create"]
    api_base: str
    status: int
    create: dict[str, JsonValue]
    reply: dict[str, JsonValue]


class ContainersListCase(TypedDict):
    case: Literal["containers_list"]
    api_base: str
    after: str | None
    limit: int | None
    order: str | None
    extra_query: dict[str, JsonValue] | None


class EvalsGetCase(TypedDict):
    case: Literal["evals_get"]
    api_base: str
    eval_id: str


class VectorStoresCreateCase(TypedDict):
    case: Literal["vector_stores_create"]
    api_base: str
    params: VectorStoreCreateOptionalRequestParams


Input: TypeAlias = (
    FilesCase
    | BatchesCase
    | FineTuningCase
    | ContainersListCase
    | EvalsGetCase
    | VectorStoresCreateCase
)


class Answer:
    def __init__(self, status: int, body: dict[str, JsonValue]) -> None:
        self.status = status
        self.body = body

    def __call__(self, request: httpx.Request) -> httpx.Response:
        return httpx.Response(self.status, json=self.body, request=request)


def async_client(api_base: str, answer: Answer) -> AsyncOpenAI:
    http_client = httpx.AsyncClient(transport=httpx.MockTransport(answer))
    return AsyncOpenAI(
        api_key="sk-fixture", base_url=api_base, max_retries=0, http_client=http_client
    )


def files_create(case: Input) -> JsonValue:
    if case["case"] != "files_create":
        raise ValueError("the call is not files_create")
    client = async_client(case["api_base"], Answer(case["status"], case["reply"]))
    data: CreateFileRequest = {
        "file": (case["filename"], case["content"].encode()),
        "purpose": "batch",
    }
    reply = asyncio.run(
        OpenAIFilesAPI().acreate_file(create_file_data=data, openai_client=client)
    )
    return {"result": JSON.validate_json(reply.model_dump_json(exclude_none=True))}


def batches_create(case: Input) -> JsonValue:
    if case["case"] != "batches_create":
        raise ValueError("the call is not batches_create")
    client = async_client(case["api_base"], Answer(case["status"], case["reply"]))
    batch = asyncio.run(
        OpenAIBatchesAPI().acreate_batch(
            create_batch_data=case["create"], openai_client=client
        )
    )
    result: dict[str, JsonValue] = {
        "id": batch.id,
        "object": batch.object,
        "endpoint": batch.endpoint,
        "completion_window": batch.completion_window,
        "status": batch.status,
        "input_file_id": batch.input_file_id,
        "metadata": JSON.validate_python(batch.metadata),
    }
    return {"result": result}


# ast-grep-ignore: no-long-names
def fine_tuning_create(case: Input) -> JsonValue:
    if case["case"] != "fine_tuning_create":
        raise ValueError("the call is not fine_tuning_create")
    client = async_client(case["api_base"], Answer(case["status"], case["reply"]))
    job = asyncio.run(
        OpenAIFineTuningAPI().acreate_fine_tuning_job(
            create_fine_tuning_job_data=case["create"], openai_client=client
        )
    )
    result: dict[str, JsonValue] = {
        "id": job.id,
        "object": job.object,
        "status": job.status,
        "model": job.model,
        "training_file": job.training_file,
        "organization_id": job.organization_id,
        "seed": job.seed,
    }
    return {"result": result}


def containers_list(case: Input) -> JsonValue:
    if case["case"] != "containers_list":
        raise ValueError("the call is not containers_list")
    config: ContainerConfig = container_config()
    url, query = config.transform_container_list_request(
        case["api_base"],
        GenericLiteLLMParams(),
        {},
        case["after"],
        case["limit"],
        case["order"],
        case["extra_query"],
    )
    return {"url": url, "query": query}


def evals_get(case: Input) -> JsonValue:
    if case["case"] != "evals_get":
        raise ValueError("the call is not evals_get")
    url, headers = EVALS.transform_get_eval_request(
        case["eval_id"], case["api_base"], GenericLiteLLMParams(), {}
    )
    return {"url": url, "headers": headers}


def vector_stores_create(case: Input) -> JsonValue:
    if case["case"] != "vector_stores_create":
        raise ValueError("the call is not vector_stores_create")
    url, body = VECTOR.transform_create_vector_store_request(case["params"], case["api_base"])
    return {"url": url, "body": body}


HANDLERS: Final[dict[str, Callable[[Input], JsonValue]]] = {
    "files_create": files_create,
    "batches_create": batches_create,
    "fine_tuning_create": fine_tuning_create,
    "containers_list": containers_list,
    "evals_get": evals_get,
    "vector_stores_create": vector_stores_create,
}


def run(input_value: Input) -> JsonValue:
    handler = HANDLERS[input_value["case"]]
    return handler(input_value)
