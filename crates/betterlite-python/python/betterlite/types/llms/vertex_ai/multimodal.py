from typing import Required

from pydantic import JsonValue
from typing_extensions import TypedDict


class VideoSegmentConfig(TypedDict, total=False):
    startOffsetSec: int
    endOffsetSec: int
    intervalSec: int


class InstanceVideo(TypedDict, total=False):
    gcsUri: str
    videoSegmentConfig: VideoSegmentConfig


class InstanceImage(TypedDict, total=False):
    gcsUri: str | None
    bytesBase64Encoded: str | None
    mimeType: str | None


class Instance(TypedDict, total=False):
    text: str
    image: InstanceImage
    video: InstanceVideo


# ast-grep-ignore: no-long-names
class VertexMultimodalEmbeddingRequest(TypedDict, total=False):
    instances: Required[list[Instance]]
    parameters: dict[str, JsonValue]


class VideoEmbedding(TypedDict):
    startOffsetSec: int
    endOffsetSec: int
    embedding: list[float]


class MultimodalPrediction(TypedDict, total=False):
    textEmbedding: list[float]
    imageEmbedding: list[float]
    videoEmbeddings: list[VideoEmbedding]


class MultimodalPredictions(TypedDict):
    predictions: list[MultimodalPrediction]
