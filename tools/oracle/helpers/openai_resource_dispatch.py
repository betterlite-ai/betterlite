import asyncio
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.llms.openai.fine_tuning.handler import OpenAIFineTuningAPI
from litellm.llms.openai.openai import OpenAIBatchesAPI, OpenAIFilesAPI
from litellm.llms.openai.vector_stores.transformation import OpenAIVectorStoreConfig
from litellm.types.llms.openai import CreateBatchRequest, CreateFileRequest
from litellm.types.router import GenericLiteLLMParams
from litellm.types.vector_stores import VectorStoreCreateOptionalRequestParams as VectorStoreParams
from openai import AsyncOpenAI
from pydantic import JsonValue, TypeAdapter

from openai_containers import ContainerConfig, container_config
from openai_evals.config import CONFIG as EVALS

JSON: Final = TypeAdapter(JsonValue)


class VectorConfig(Protocol):
    # ast-grep-ignore: no-long-names
    def transform_create_vector_store_request(
        self,
        # ast-grep-ignore: no-long-names
        vector_store_create_optional_params: VectorStoreParams,
        api_base: str,
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
    params: VectorStoreParams


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


def files_create(case: FilesCase) -> JsonValue:
    client = async_client(case["api_base"], Answer(case["status"], case["reply"]))
    data: CreateFileRequest = {
        "file": (case["filename"], case["content"].encode()),
        "purpose": "batch",
    }
    reply = asyncio.run(
        OpenAIFilesAPI().acreate_file(create_file_data=data, openai_client=client)
    )
    return {"result": JSON.validate_json(reply.model_dump_json(exclude_none=True))}


def batches_create(case: BatchesCase) -> JsonValue:
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


def fine_tuning_create(case: FineTuningCase) -> JsonValue:
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


def containers_list(case: ContainersListCase) -> JsonValue:
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


def evals_get(case: EvalsGetCase) -> JsonValue:
    url, headers = EVALS.transform_get_eval_request(
        case["eval_id"], case["api_base"], GenericLiteLLMParams(), {}
    )
    return {"url": url, "headers": headers}


def vector_stores_create(case: VectorStoresCreateCase) -> JsonValue:
    url, body = VECTOR.transform_create_vector_store_request(case["params"], case["api_base"])
    return {"url": url, "body": body}


def run(input_value: Input) -> JsonValue:
    match input_value["case"]:
        case "files_create":
            return files_create(input_value)
        case "batches_create":
            return batches_create(input_value)
        case "fine_tuning_create":
            return fine_tuning_create(input_value)
        case "containers_list":
            return containers_list(input_value)
        case "evals_get":
            return evals_get(input_value)
        case "vector_stores_create":
            return vector_stores_create(input_value)
