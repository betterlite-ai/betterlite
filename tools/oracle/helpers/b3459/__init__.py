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
    operations: list[JsonValue]
    answer: list[list[float]]
    failing: bool

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
        input: str | list[str],  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L5933
        is_async: bool | None = True,  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L5934
        **_kwargs: JsonValue,
    ) -> EmbeddingResponse:
        texts: list[str] = []
        match input:
            case str():
                texts = [input]
            case list():
                texts = input
        self.operations.append(
            _JSON.validate_python({"call": "embedding", "model": model, "input": texts})
        )
        if self.failing:
            raise RuntimeError("scripted embedding failure")
        data = [
            Embedding(embedding=vector, index=index, object="embedding")
            for index, vector in enumerate(self.answer)
        ]
        return EmbeddingResponse(model=model, data=data)


class RecordingIngestion(S3VectorsRAGIngestion):
    operations: list[JsonValue]

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
        self.operations.append(
            _JSON.validate_python({"call": "ensure_bucket", "bucket": self.vector_bucket_name})
        )

    @override
    async def _ensure_vector_index_exists(self) -> None:
        self.operations.append(
            _JSON.validate_python(
                {
                    "call": "ensure_index",
                    "bucket": self.vector_bucket_name,
                    "index": self.index_name,
                    "dimension": self.dimension,
                    "distance_metric": self.distance_metric,
                    "non_filterable_metadata_keys": list(self.non_filterable_metadata_keys),
                }
            )
        )

    @override
    async def _put_vectors(self, vectors: Sequence[S3VectorEntry]) -> None:
        recorded = [
            {
                "key": entry["key"],
                "data": {"float32": list(entry["data"]["float32"])},
                "metadata": dict(entry["metadata"]),
            }
            for entry in vectors
        ]
        self.operations.append(
            _JSON.validate_python(
                {
                    "call": "put_vectors",
                    "bucket": self.vector_bucket_name,
                    "index": self.index_name,
                    "vectors": recorded,
                }
            )
        )


def settings_for(case: SettingsCase) -> JsonValue:
    router = ScriptedRouter([], None, failing=False)
    try:
        ingestion = RecordingIngestion(
            case["vector"], case.get("embedding"), case["environment"], router
        )
    except ValueError as error:
        return _JSON.validate_python({"failed": str(error)})
    embedding_config = ingestion.embedding_config
    return _JSON.validate_python(
        {
            "vector_bucket_name": ingestion.vector_bucket_name,
            "index_name": ingestion.index_name,
            "embedding_model": None if embedding_config is None else embedding_config.get("model"),
            "distance_metric": ingestion.distance_metric,
            "non_filterable_metadata_keys": list(ingestion.non_filterable_metadata_keys),
            "dimension": ingestion.dimension,
            "aws_region_name": ingestion.aws_region_name,
        }
    )


def dimension_for(case: DimensionCase) -> JsonValue:
    router = ScriptedRouter([], case["answer"], failing=case["failing"])
    ingestion = RecordingIngestion(
        case["vector"], case.get("embedding"), case["environment"], router
    )
    asyncio.run(ingestion.store(None, "a.txt", None, ["one"], [[0.5]]))
    return _JSON.validate_python({"operations": router.operations})


def flow_for(case: FlowCase) -> JsonValue:
    router = ScriptedRouter([], case.get("answer"), failing=False)
    ingestion = RecordingIngestion(
        case["vector"], case.get("embedding"), case["environment"], router
    )
    vector_store_id, filename = asyncio.run(
        ingestion.store(None, case.get("filename"), None, case["chunks"], case["embeddings"])
    )
    return _JSON.validate_python(
        {
            "result": {"vector_store_id": vector_store_id, "filename": filename},
            "operations": router.operations,
        }
    )


def embed_step(input_value: EmbedInput) -> JsonValue:
    router = ScriptedRouter([], input_value["answer"], failing=False)
    ingestion = RecordingIngestion(
        input_value["vector"], input_value.get("embedding"), input_value["environment"], router
    )
    results: list[JsonValue] = [
        _JSON.validate_python(asyncio.run(ingestion.embed(chunks)))
        for chunks in input_value["chunk_sets"]
    ]
    return _JSON.validate_python({"results": results, "operations": router.operations})


def store_call(ingestion: RecordingIngestion, call: StoreCall) -> JsonValue:
    try:
        vector_store_id, filename = asyncio.run(
            ingestion.store(None, call.get("filename"), None, call["chunks"], call.get("embeddings"))
        )
    except ValueError as error:
        return _JSON.validate_python({"failed": str(error)})
    return _JSON.validate_python({"vector_store_id": vector_store_id, "filename": filename})


def store_step(input_value: StoreInput) -> JsonValue:
    router = ScriptedRouter([], None, failing=False)
    ingestion = RecordingIngestion(
        input_value["vector"], input_value.get("embedding"), input_value["environment"], router
    )
    results: list[JsonValue] = [store_call(ingestion, call) for call in input_value["calls"]]
    return _JSON.validate_python({"results": results, "operations": router.operations})


def run(input_value: Input) -> JsonValue:
    if input_value["step"] == "settings":
        return _JSON.validate_python(
            {"settings": [settings_for(case) for case in input_value["cases"]]}
        )
    if input_value["step"] == "dimension":
        return _JSON.validate_python(
            {"cases": [dimension_for(case) for case in input_value["cases"]]}
        )
    if input_value["step"] == "flow":
        return _JSON.validate_python({"cases": [flow_for(case) for case in input_value["cases"]]})
    if input_value["step"] == "embed":
        return embed_step(input_value)
    return store_step(input_value)
