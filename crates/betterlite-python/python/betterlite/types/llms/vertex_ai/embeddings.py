from enum import Enum
from typing import Required, TypeAlias

from typing_extensions import TypedDict

from betterlite.types.llms.openai.core.aliases import EmbeddingInput
from betterlite.types.llms.vertex_ai.content import ContentType

GeminiEmbeddingInput: TypeAlias = EmbeddingInput | list[list[str]]


class TaskTypeEnum(Enum):
    TASK_TYPE_UNSPECIFIED = "TASK_TYPE_UNSPECIFIED"
    RETRIEVAL_QUERY = "RETRIEVAL_QUERY"
    RETRIEVAL_DOCUMENT = "RETRIEVAL_DOCUMENT"
    SEMANTIC_SIMILARITY = "SEMANTIC_SIMILARITY"
    CLASSIFICATION = "CLASSIFICATION"
    CLUSTERING = "CLUSTERING"
    QUESTION_ANSWERING = "QUESTION_ANSWERING"
    FACT_VERIFICATION = "FACT_VERIFICATION"


# ast-grep-ignore: no-long-names
class VertexAITextEmbeddingsRequestBody(TypedDict, total=False):
    content: Required[ContentType]
    taskType: TaskTypeEnum
    title: str
    outputDimensionality: int


class ContentEmbeddings(TypedDict):
    values: list[int]


# ast-grep-ignore: no-long-names
class VertexAITextEmbeddingsResponseObject(TypedDict):
    embedding: ContentEmbeddings


# ast-grep-ignore: no-long-names
class EmbedContentRequest(VertexAITextEmbeddingsRequestBody):
    model: Required[str]


# ast-grep-ignore: no-long-names
class VertexAIBatchEmbeddingsRequestBody(TypedDict, total=False):
    requests: list[EmbedContentRequest]


# ast-grep-ignore: no-long-names
class VertexAIBatchEmbeddingsResponseObject(TypedDict):
    embeddings: list[ContentEmbeddings]


class GeminiEmbedContentRequestBody(TypedDict, total=False):
    content: Required[ContentType]
    taskType: TaskTypeEnum
    title: str
    outputDimensionality: int


# ast-grep-ignore: no-long-names
class GeminiEmbedContentResponseObject(TypedDict):
    embedding: ContentEmbeddings
