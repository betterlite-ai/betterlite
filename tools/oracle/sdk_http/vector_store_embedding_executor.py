import asyncio
from typing import cast

from litellm.llms.base_llm.vector_store.transformation import (
    LiteLLMVectorStoreEmbeddingExecutor,
    RouterVectorStoreEmbeddingExecutor,
    VectorStoreEmbeddingExecutor,
)
from litellm.router import Router
from litellm.types.utils import EmbeddingResponse, Usage
from pydantic import BaseModel, ConfigDict, Field


class RouterDeploymentParams(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    api_key: str


class RouterDeployment(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model_name: str
    litellm_params: RouterDeploymentParams


class EmbeddingCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    query: str
    configuration: dict[str, object] = Field(default_factory=dict)
    metadata: dict[str, object] = Field(default_factory=dict)
    router: list[RouterDeployment] | None = None


class EmbeddingRecord(BaseModel):
    object: str
    embedding: list[float]
    index: int


class EmbeddingUsage(BaseModel):
    prompt_tokens: int
    completion_tokens: int
    completion_tokens_details: dict[str, object] | None
    prompt_tokens_details: dict[str, object] | None
    total_tokens: int


class EmbeddingCompletion(BaseModel):
    object: str
    data: list[EmbeddingRecord]
    model: str
    usage: EmbeddingUsage


class EmbeddingOutcomes(BaseModel):
    sync: EmbeddingCompletion
    asynchronous: EmbeddingCompletion = Field(serialization_alias="async")


class EmbeddingSuccess(BaseModel):
    value: EmbeddingOutcomes


class EmbeddingOutcome(BaseModel):
    embedding: EmbeddingSuccess


def details(value: object) -> dict[str, object] | None:
    if isinstance(value, BaseModel):
        return cast("dict[str, object]", value.model_dump(mode="json"))
    if isinstance(value, dict):
        return cast("dict[str, object]", value)
    return None


def usage_record(usage: Usage) -> EmbeddingUsage:
    return EmbeddingUsage(
        prompt_tokens=usage.prompt_tokens,
        completion_tokens=usage.completion_tokens,
        completion_tokens_details=details(usage.completion_tokens_details),
        prompt_tokens_details=details(usage.prompt_tokens_details),
        total_tokens=usage.total_tokens,
    )


def project(response: EmbeddingResponse) -> EmbeddingCompletion:
    return EmbeddingCompletion(
        object=response.object,
        data=[
            EmbeddingRecord(
                object=cast("str", record["object"]),
                embedding=[
                    float(value) for value in cast("list[float]", record["embedding"])
                ],
                index=cast("int", record["index"]),
            )
            for record in cast("list[dict[str, object]]", response.data)
        ],
        model=cast("str", response.model),
        usage=usage_record(cast("Usage", response.usage)),
    )


def build_executor(call: EmbeddingCall, api_base: str) -> VectorStoreEmbeddingExecutor:
    if call.router is None:
        return LiteLLMVectorStoreEmbeddingExecutor()
    model_list: list[dict[str, object]] = [
        {
            "model_name": deployment.model_name,
            "litellm_params": {
                "model": deployment.litellm_params.model,
                "api_key": deployment.litellm_params.api_key,
                "api_base": api_base,
            },
        }
        for deployment in call.router
    ]
    return RouterVectorStoreEmbeddingExecutor(
        router=Router(model_list=model_list), metadata=call.metadata
    )


def run_embedding(call: EmbeddingCall, api_base: str) -> EmbeddingOutcome:
    configuration: dict[str, object] = {**call.configuration, "api_base": api_base}
    executor = build_executor(call, api_base)
    sync = executor.embed(
        model=call.model, query=call.query, configuration=configuration
    )
    asynchronous = asyncio.run(
        executor.aembed(model=call.model, query=call.query, configuration=configuration)
    )
    return EmbeddingOutcome(
        embedding=EmbeddingSuccess(
            value=EmbeddingOutcomes(sync=project(sync), asynchronous=project(asynchronous))
        )
    )
