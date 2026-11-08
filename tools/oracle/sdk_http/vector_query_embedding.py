import asyncio
from typing import cast

from litellm.llms.base_llm.vector_store.transformation import BaseQueryEmbeddingVectorStoreConfig
from pydantic import BaseModel, ConfigDict, Field


class QueryEmbeddingCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    query_text: str
    litellm_params: dict[str, object] = Field(default_factory=dict)


class QueryVectors(BaseModel):
    sync: list[float]
    asynchronous: list[float] = Field(serialization_alias="async")


class QueryVectorsValue(BaseModel):
    value: QueryVectors


class Failure(BaseModel):
    class_name: str = Field(serialization_alias="class")
    status: int
    type: str | None = None
    param: str | None = None
    code: str | None = None
    message: str


class QueryOutcome(BaseModel):
    vectors: QueryVectorsValue | None = None
    failure: Failure | None = None


def config() -> BaseQueryEmbeddingVectorStoreConfig:
    derived = type("Concrete", (BaseQueryEmbeddingVectorStoreConfig,), {})
    setattr(derived, "__abstractmethods__", frozenset())  # noqa: B010 - type has no typed attribute for it
    return cast("BaseQueryEmbeddingVectorStoreConfig", derived())


def project(error: Exception) -> Failure:
    # a plain Python exception carries no HTTP status, type, param or code
    return Failure(class_name=type(error).__name__, status=0, message=str(error))


def run_query(call: QueryEmbeddingCall, api_base: str) -> QueryOutcome:
    fixture = config()
    litellm_params = dict(call.litellm_params)
    configuration = litellm_params.get("litellm_embedding_config")
    if isinstance(configuration, dict):
        litellm_params["litellm_embedding_config"] = {
            **cast("dict[str, object]", configuration),
            "api_base": api_base,
        }
    try:
        sync = fixture.embed_query(call.query_text, litellm_params, None)
        asynchronous = asyncio.run(fixture.aembed_query(call.query_text, litellm_params, None))
    except Exception as error:  # noqa: BLE001 - LiteLLM raises a bare exception around the executor call
        return QueryOutcome(failure=project(error))
    return QueryOutcome(
        vectors=QueryVectorsValue(
            value=QueryVectors(sync=list(sync), asynchronous=list(asynchronous))
        )
    )
