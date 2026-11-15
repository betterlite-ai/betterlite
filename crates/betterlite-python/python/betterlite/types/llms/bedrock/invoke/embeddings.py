from typing import Literal, Required

from typing_extensions import TypedDict

COHERE_EMBEDDING_INPUT_TYPES = Literal[
    "search_document", "search_query", "classification", "clustering", "image",
]


class CohereEmbeddingRequest(TypedDict, total=False):
    texts: list[str]
    images: list[str]
    input_type: Required[COHERE_EMBEDDING_INPUT_TYPES]
    truncate: Literal["NONE", "START", "END"]
    embedding_types: Literal["float", "int8", "uint8", "binary", "ubinary"]
    output_dimension: int


class CohereEmbeddingRequestWithModel(CohereEmbeddingRequest):  # ast-grep-ignore: no-long-names
    model: Required[str]


class CohereEmbeddingResponse(TypedDict):
    embeddings: list[list[float]]
    id: str
    response_type: Literal["embedding_floats"]
    texts: list[str]


class AmazonTitanV2EmbeddingRequest(TypedDict, total=False):
    inputText: Required[str]
    dimensions: int
    normalize: bool
    embeddingTypes: list[Literal["float", "binary"]]


class AmazonTitanV2EmbeddingsByType(TypedDict, total=False):
    binary: list[int]
    float: list[float]


class AmazonTitanV2EmbeddingResponse(TypedDict, total=False):
    embedding: list[float]
    embeddingsByType: AmazonTitanV2EmbeddingsByType
    inputTextTokenCount: Required[int]


class AmazonTitanG1EmbeddingRequest(TypedDict):
    inputText: str


class AmazonTitanG1EmbeddingResponse(TypedDict):
    embedding: list[float]
    inputTextTokenCount: int


class AmazonTitanMultimodalEmbeddingConfig(TypedDict):  # ast-grep-ignore: no-long-names
    outputEmbeddingLength: Literal[256, 384, 1024]


# ast-grep-ignore: no-long-names
class AmazonTitanMultimodalEmbeddingRequest(TypedDict, total=False):
    inputText: str
    inputImage: str
    # ast-grep-ignore: no-long-names
    embeddingConfig: AmazonTitanMultimodalEmbeddingConfig


class AmazonTitanMultimodalEmbeddingResponse(TypedDict):  # ast-grep-ignore: no-long-names
    embedding: list[float]
    inputTextTokenCount: int
    message: str
