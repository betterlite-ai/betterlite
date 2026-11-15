import asyncio

from litellm.llms.base_llm.vector_store.transformation import (
    BaseQueryEmbeddingVectorStoreConfig as QueryEmbeddingBase,
)
from pydantic import BaseModel, ConfigDict, Field, JsonValue


class QueryEmbeddingCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    query_text: str
    litellm_params: dict[str, JsonValue] = Field(default_factory=dict)


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


class QueryEmbeddingConfig(QueryEmbeddingBase):
    __abstractmethods__: frozenset[str] = frozenset()


# LiteLLM's embed_query and aembed_query only reach the base bodies of this config
CONFIG: QueryEmbeddingBase = QueryEmbeddingConfig()  # type: ignore[abstract]  # pyright: ignore[reportAbstractUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/vector_store/transformation.py#L260-L273


def run_query(call: QueryEmbeddingCall, api_base: str) -> QueryOutcome:
    litellm_params = dict(call.litellm_params)
    configuration = litellm_params.get("litellm_embedding_config")
    match configuration:
        case dict() as config_map:
            litellm_params["litellm_embedding_config"] = {
                **config_map,
                "api_base": api_base,
            }
        case str() | bool() | int() | float() | list() | None:
            pass
    try:
        sync = CONFIG.embed_query(call.query_text, litellm_params, None)
        asynchronous = asyncio.run(
            CONFIG.aembed_query(call.query_text, litellm_params, None)
        )
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/vector_store/transformation.py#L330-L357
        return QueryOutcome(
            failure=Failure(
                class_name=type(error).__name__, status=0, message=str(error)
            )
        )
    return QueryOutcome(
        vectors=QueryVectorsValue(
            value=QueryVectors(sync=list(sync), asynchronous=list(asynchronous))
        )
    )
