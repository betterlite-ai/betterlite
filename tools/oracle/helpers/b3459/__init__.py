import asyncio
import os
from collections.abc import Sequence
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.rag.ingestion.s3_vectors_ingestion import S3VectorEntry, S3VectorsRAGIngestion
from litellm.router import Router
from litellm.types.rag import (
    RAGIngestEmbeddingOptions,
    RAGIngestOptions,
    S3VectorsVectorStoreOptions,
)
from litellm.types.utils import Embedding, EmbeddingResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

_JSON: Final = TypeAdapter(JsonValue)


class SettingsCase(TypedDict):
    vector: S3VectorsVectorStoreOptions
    environment: dict[str, str]
    embedding: NotRequired[RAGIngestEmbeddingOptions]


class SettingsInput(TypedDict):
    step: Literal["settings"]
    cases: list[SettingsCase]


class DimensionCase(TypedDict):
    vector: S3VectorsVectorStoreOptions
    environment: dict[str, str]
    embedding: NotRequired[RAGIngestEmbeddingOptions]
    answer: list[list[float]]
    failing: bool


class DimensionInput(TypedDict):
    step: Literal["dimension"]
    cases: list[DimensionCase]


class FlowCase(TypedDict):
    vector: S3VectorsVectorStoreOptions
    environment: dict[str, str]
    embedding: NotRequired[RAGIngestEmbeddingOptions]
    answer: NotRequired[list[list[float]]]
    filename: NotRequired[str | None]
    chunks: list[str]
    embeddings: list[list[float]]


class FlowInput(TypedDict):
    step: Literal["flow"]
    cases: list[FlowCase]


class EmbedInput(TypedDict):
    step: Literal["embed"]
    vector: S3VectorsVectorStoreOptions
    environment: dict[str, str]
    embedding: NotRequired[RAGIngestEmbeddingOptions]
    chunk_sets: list[list[str]]
    answer: list[list[float]]


class StoreCall(TypedDict):
    filename: NotRequired[str | None]
    chunks: list[str]
    embeddings: NotRequired[list[list[float]]]


class StoreInput(TypedDict):
    step: Literal["store"]
    vector: S3VectorsVectorStoreOptions
    environment: dict[str, str]
    embedding: NotRequired[RAGIngestEmbeddingOptions]
    calls: list[StoreCall]


Input: TypeAlias = SettingsInput | DimensionInput | FlowInput | EmbedInput | StoreInput


class ScriptedRouter(Router):
    def __init__(
        self,
        operations: list[JsonValue],
        answer: list[list[float]] | None,
        *,
        failing: bool,
    ) -> None:
        # the ingestion pipeline uses the router only for aembedding, so skip Router.__init__
        object.__init__(self)
        self.operations = operations
        self.answer = [] if answer is None else answer
        self.failing = failing

    @override
    async def aembedding(
        self,
        model: str,
        input: str | list[str],  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L5933
        is_async: bool | None = True,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L5934
        **_kwargs: JsonValue,
    ) -> EmbeddingResponse:
        texts: list[str] = []
        match input:
            case str():
                texts = [input]
            case list():
                texts = input
        recorded = {"call": "embedding", "model": model, "input": texts}
        self.operations.append(_JSON.validate_python(recorded))
        if self.failing:
            raise RuntimeError("scripted embedding failure")
        data = [
            Embedding(embedding=vector, index=index, object="embedding")
            for index, vector in enumerate(self.answer)
        ]
        return EmbeddingResponse(model=model, data=data)


class RecordingIngestion(S3VectorsRAGIngestion):
    def __init__(
        self,
        vector: S3VectorsVectorStoreOptions,
        embedding: RAGIngestEmbeddingOptions | None,
        environment: dict[str, str],
        router: ScriptedRouter,
    ) -> None:
        self.operations = router.operations
        saved = dict(os.environ)
        os.environ.clear()
        os.environ.update(environment)
        try:
            options = RAGIngestOptions(vector_store=vector)
            if embedding is not None:
                options["embedding"] = embedding
            super().__init__(ingest_options=options, router=router)
        finally:
            os.environ.clear()
            os.environ.update(saved)

    @override
    async def _ensure_vector_bucket_exists(self) -> None:
        self.operations.append({"call": "ensure_bucket", "bucket": self.vector_bucket_name})

    @override
    async def _ensure_vector_index_exists(self) -> None:
        self.operations.append(
            {
                "call": "ensure_index",
                "bucket": self.vector_bucket_name,
                "index": self.index_name,
                "dimension": self.dimension,
                "distance_metric": self.distance_metric,
                "non_filterable_metadata_keys": list(self.non_filterable_metadata_keys),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rag/ingestion/s3_vectors_ingestion.py#L99-L102
            }
        )

    @override
    async def _put_vectors(self, vectors: Sequence[S3VectorEntry]) -> None:
        stored = [
            {
                "key": entry["key"],
                "data": {"float32": list(entry["data"]["float32"])},
                "metadata": dict(entry["metadata"]),
            }
            for entry in vectors
        ]
        recorded = {
            "call": "put_vectors",
            "bucket": self.vector_bucket_name,
            "index": self.index_name,
            "vectors": stored,
        }
        self.operations.append(_JSON.validate_python(recorded))


def build_ingestion(
    vector: S3VectorsVectorStoreOptions,
    embedding: RAGIngestEmbeddingOptions | None,
    environment: dict[str, str],
    answer: list[list[float]] | None,
    *,
    failing: bool = False,
) -> RecordingIngestion:
    router = ScriptedRouter([], answer, failing=failing)
    return RecordingIngestion(vector, embedding, environment, router)


def settings_for(case: SettingsCase) -> JsonValue:
    try:
        ingestion = build_ingestion(
            case["vector"], case.get("embedding"), case["environment"], None
        )
    except ValueError as error:
        return {"failed": str(error)}
    embedding_config = ingestion.embedding_config
    return {
        "vector_bucket_name": ingestion.vector_bucket_name,
        "index_name": ingestion.index_name,
        "embedding_model": None if embedding_config is None else embedding_config.get("model"),
        "distance_metric": ingestion.distance_metric,
        "non_filterable_metadata_keys": list(ingestion.non_filterable_metadata_keys),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rag/ingestion/s3_vectors_ingestion.py#L99-L102
        "dimension": ingestion.dimension,
        "aws_region_name": ingestion.aws_region_name,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L845-L848
    }


def dimension_for(case: DimensionCase) -> JsonValue:
    ingestion = build_ingestion(
        case["vector"],
        case.get("embedding"),
        case["environment"],
        case["answer"],
        failing=case["failing"],
    )
    asyncio.run(ingestion.store(None, "a.txt", None, ["one"], [[0.5]]))
    return {"operations": ingestion.operations}


def flow_for(case: FlowCase) -> JsonValue:
    ingestion = build_ingestion(
        case["vector"], case.get("embedding"), case["environment"], case.get("answer")
    )
    requested_name = case.get("filename")
    vector_store_id, stored_name = asyncio.run(
        ingestion.store(None, requested_name, None, case["chunks"], case["embeddings"])
    )
    return {
        "result": {"vector_store_id": vector_store_id, "filename": stored_name},
        "operations": ingestion.operations,
    }


def embed_step(input_value: EmbedInput) -> JsonValue:
    ingestion = build_ingestion(
        input_value["vector"],
        input_value.get("embedding"),
        input_value["environment"],
        input_value["answer"],
    )
    results: list[JsonValue] = [
        _JSON.validate_python(asyncio.run(ingestion.embed(chunks)))
        for chunks in input_value["chunk_sets"]
    ]
    return {"results": results, "operations": ingestion.operations}


def store_call(ingestion: RecordingIngestion, call: StoreCall) -> JsonValue:
    try:
        vector_store_id, filename = asyncio.run(
            ingestion.store(
                None, call.get("filename"), None, call["chunks"], call.get("embeddings")
            )
        )
    except ValueError as error:
        return {"failed": str(error)}
    return {"vector_store_id": vector_store_id, "filename": filename}


def store_step(input_value: StoreInput) -> JsonValue:
    ingestion = build_ingestion(
        input_value["vector"], input_value.get("embedding"), input_value["environment"], None
    )
    results: list[JsonValue] = [store_call(ingestion, call) for call in input_value["calls"]]
    return {"results": results, "operations": ingestion.operations}


def run(input_value: Input) -> JsonValue:
    if input_value["step"] == "settings":
        return {"settings": [settings_for(case) for case in input_value["cases"]]}
    if input_value["step"] == "dimension":
        return {"cases": [dimension_for(case) for case in input_value["cases"]]}
    if input_value["step"] == "flow":
        return {"cases": [flow_for(case) for case in input_value["cases"]]}
    if input_value["step"] == "embed":
        return embed_step(input_value)
    return store_step(input_value)
