from typing import Literal, TypeAlias

from typing_extensions import Required, TypedDict

from betterlite.types.llms.bedrock.invoke.embeddings import (
    AmazonTitanG1EmbeddingRequest,
    AmazonTitanMultimodalEmbeddingRequest,  # ast-grep-ignore: no-long-names
    AmazonTitanV2EmbeddingRequest,
)

NOVA_EMBEDDING_PURPOSES = Literal[
    "GENERIC_INDEX",
    "GENERIC_RETRIEVAL",
    "TEXT_RETRIEVAL",
    "IMAGE_RETRIEVAL",
    "VIDEO_RETRIEVAL",
    "DOCUMENT_RETRIEVAL",
    "AUDIO_RETRIEVAL",
    "CLASSIFICATION",
    "CLUSTERING",
]

NOVA_EMBEDDING_DIMENSIONS = Literal[256, 384, 1024, 3072]

NOVA_TRUNCATION_MODES = Literal["START", "END", "NONE"]

NOVA_DETAIL_LEVELS = Literal["STANDARD_IMAGE", "DOCUMENT_IMAGE"]

NOVA_EMBEDDING_MODES = Literal["AUDIO_VIDEO_COMBINED", "AUDIO_VIDEO_SEPARATE"]

NOVA_EMBEDDING_TYPES = Literal["TEXT", "IMAGE", "VIDEO", "AUDIO", "AUDIO_VIDEO_COMBINED"]


class NovaSourceS3Location(TypedDict):
    uri: str


class NovaSourceObject(TypedDict, total=False):
    bytes: str
    s3Location: NovaSourceS3Location


class NovaTextParams(TypedDict, total=False):
    truncationMode: NOVA_TRUNCATION_MODES
    value: str
    source: NovaSourceObject


class NovaImageParams(TypedDict, total=False):
    format: str
    source: Required[NovaSourceObject]
    detailLevel: NOVA_DETAIL_LEVELS


class NovaVideoParams(TypedDict, total=False):
    format: str
    source: Required[NovaSourceObject]
    embeddingMode: Required[NOVA_EMBEDDING_MODES]


class NovaAudioParams(TypedDict, total=False):
    format: str
    source: Required[NovaSourceObject]


class NovaTextSegmentationConfig(TypedDict, total=False):
    maxLengthChars: int


class NovaMediaSegmentationConfig(TypedDict, total=False):
    durationSeconds: int


class NovaTextParamsWithSegmentation(NovaTextParams, total=False):
    segmentationConfig: NovaTextSegmentationConfig


# ast-grep-ignore: no-long-names
class NovaVideoParamsWithSegmentation(NovaVideoParams, total=False):
    segmentationConfig: NovaMediaSegmentationConfig


# ast-grep-ignore: no-long-names
class NovaAudioParamsWithSegmentation(NovaAudioParams, total=False):
    segmentationConfig: NovaMediaSegmentationConfig


class NovaSingleEmbeddingParams(TypedDict, total=False):
    embeddingPurpose: Required[NOVA_EMBEDDING_PURPOSES]
    embeddingDimension: NOVA_EMBEDDING_DIMENSIONS
    text: NovaTextParams
    image: NovaImageParams
    video: NovaVideoParams
    audio: NovaAudioParams


class NovaSegmentedEmbeddingParams(TypedDict, total=False):
    embeddingPurpose: Required[NOVA_EMBEDDING_PURPOSES]
    embeddingDimension: NOVA_EMBEDDING_DIMENSIONS
    # ast-grep-ignore: no-long-names
    text: NovaTextParamsWithSegmentation
    image: NovaImageParams
    # ast-grep-ignore: no-long-names
    video: NovaVideoParamsWithSegmentation
    # ast-grep-ignore: no-long-names
    audio: NovaAudioParamsWithSegmentation


class NovaEmbeddingRequest(TypedDict, total=False):
    schemaVersion: str
    taskType: Literal["SINGLE_EMBEDDING", "SEGMENTED_EMBEDDING"]
    singleEmbeddingParams: NovaSingleEmbeddingParams
    segmentedEmbeddingParams: NovaSegmentedEmbeddingParams


class NovaEmbeddingItem(TypedDict, total=False):
    embeddingType: NOVA_EMBEDDING_TYPES
    embedding: Required[list[float]]
    truncatedCharLength: int


class NovaEmbeddingResponse(TypedDict):
    embeddings: list[NovaEmbeddingItem]


class NovaS3OutputDataConfig(TypedDict):
    s3Uri: str


class NovaOutputDataConfig(TypedDict):
    s3OutputDataConfig: NovaS3OutputDataConfig


class NovaAsyncInvokeRequest(TypedDict):
    modelId: str
    modelInput: NovaEmbeddingRequest
    outputDataConfig: NovaOutputDataConfig


AmazonEmbeddingRequest: TypeAlias = (
    # ast-grep-ignore: no-long-names
    AmazonTitanMultimodalEmbeddingRequest
    | AmazonTitanV2EmbeddingRequest
    | AmazonTitanG1EmbeddingRequest
)
