from collections.abc import Sequence
from typing import Literal, Required, TypeAlias

from typing_extensions import ReadOnly, TypedDict

# ast-grep-ignore: no-long-names
TWELVELABS_EMBEDDING_INPUT_TYPES = Literal["text", "image", "video", "audio"]
TWELVELABS_EMBEDDING_OPTIONS = Literal["visual-text", "visual-image", "audio"]


class TwelveLabsS3Location(TypedDict, total=False):
    uri: str
    bucketOwner: str


class TwelveLabsMediaSource(TypedDict, total=False):
    base64String: str
    s3Location: TwelveLabsS3Location


# ast-grep-ignore: no-long-names
class TwelveLabsMarengoEmbeddingRequest(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    inputType: Required[TWELVELABS_EMBEDDING_INPUT_TYPES]
    inputText: str
    mediaSource: TwelveLabsMediaSource
    textTruncate: Literal["end", "none"]
    startSec: float
    lengthSec: float
    useFixedLengthSec: float
    minClipSec: int
    embeddingOption: list[TWELVELABS_EMBEDDING_OPTIONS]


class TwelveLabsMarengoEmbeddingResponse(TypedDict):  # ast-grep-ignore: no-long-names
    embedding: list[float]
    embeddingOption: TWELVELABS_EMBEDDING_OPTIONS
    startSec: float
    endSec: float


# ast-grep-ignore: no-long-names
TWELVELABS_MARENGO_3_INPUT_TYPES: TypeAlias = Literal[
    "text", "image", "video", "audio", "text_image", "multi_input",
]
# ast-grep-ignore: no-long-names
TWELVELABS_MARENGO_3_EMBEDDING_OPTIONS: TypeAlias = Literal[
    "visual", "audio", "transcription",
]
# ast-grep-ignore: no-long-names
TWELVELABS_MARENGO_3_EMBEDDING_TYPES: TypeAlias = Literal[
    "separate_embedding", "fused_embedding",
]
# ast-grep-ignore: no-long-names
TWELVELABS_MARENGO_3_EMBEDDING_SCOPES: TypeAlias = Literal["clip", "asset"]


class TwelveLabsMarengo3FixedSegmentationConfig(TypedDict):  # ast-grep-ignore: no-long-names
    durationSec: ReadOnly[int]


class TwelveLabsMarengo3FixedSegmentation(TypedDict):  # ast-grep-ignore: no-long-names
    method: ReadOnly[Literal["fixed"]]
    # ast-grep-ignore: no-long-names
    fixed: ReadOnly[TwelveLabsMarengo3FixedSegmentationConfig]


class TwelveLabsMarengo3DynamicSegmentationConfig(TypedDict):  # ast-grep-ignore: no-long-names
    minDurationSec: ReadOnly[int]


class TwelveLabsMarengo3DynamicSegmentation(TypedDict):  # ast-grep-ignore: no-long-names
    method: ReadOnly[Literal["dynamic"]]
    # ast-grep-ignore: no-long-names
    dynamic: ReadOnly[TwelveLabsMarengo3DynamicSegmentationConfig]


TwelveLabsMarengo3Segmentation: TypeAlias = (
    # ast-grep-ignore: no-long-names
    TwelveLabsMarengo3FixedSegmentation | TwelveLabsMarengo3DynamicSegmentation
)


class TwelveLabsMarengo3TextInput(TypedDict):
    inputText: ReadOnly[str]


class TwelveLabsMarengo3ImageInput(TypedDict):
    mediaSource: ReadOnly[TwelveLabsMediaSource]


# ast-grep-ignore: no-long-names
class TwelveLabsMarengo3TimedMediaOptions(TypedDict, total=False):
    startSec: ReadOnly[float]
    endSec: ReadOnly[float]
    segmentation: ReadOnly[TwelveLabsMarengo3Segmentation]
    # ast-grep-ignore: no-long-names
    embeddingOption: ReadOnly[Sequence[TWELVELABS_MARENGO_3_EMBEDDING_OPTIONS]]
    # ast-grep-ignore: no-long-names
    embeddingType: ReadOnly[Sequence[TWELVELABS_MARENGO_3_EMBEDDING_TYPES]]
    # ast-grep-ignore: no-long-names
    embeddingScope: ReadOnly[Sequence[TWELVELABS_MARENGO_3_EMBEDDING_SCOPES]]


# ast-grep-ignore: no-long-names
class TwelveLabsMarengo3TimedMediaInput(TwelveLabsMarengo3TimedMediaOptions):
    mediaSource: Required[ReadOnly[TwelveLabsMediaSource]]


class TwelveLabsMarengo3TextImageInput(TypedDict):  # ast-grep-ignore: no-long-names
    inputText: ReadOnly[str]
    mediaSource: ReadOnly[TwelveLabsMediaSource]


# ast-grep-ignore: no-long-names
class TwelveLabsMarengo3NamedMediaSource(TwelveLabsMediaSource):
    name: Required[ReadOnly[str]]
    mediaType: Required[ReadOnly[Literal["image"]]]


class TwelveLabsMarengo3MultiInput(TypedDict, total=False):
    inputText: ReadOnly[str]
    # ast-grep-ignore: no-long-names
    mediaSources: Required[ReadOnly[Sequence[TwelveLabsMarengo3NamedMediaSource]]]


class TwelveLabsMarengo3RequestBase(TypedDict, total=False):
    inferenceId: ReadOnly[str]


class TwelveLabsMarengo3TextRequest(TwelveLabsMarengo3RequestBase):
    inputType: ReadOnly[Literal["text"]]
    text: ReadOnly[TwelveLabsMarengo3TextInput]


class TwelveLabsMarengo3ImageRequest(TwelveLabsMarengo3RequestBase):
    inputType: ReadOnly[Literal["image"]]
    image: ReadOnly[TwelveLabsMarengo3ImageInput]


class TwelveLabsMarengo3VideoRequest(TwelveLabsMarengo3RequestBase):
    inputType: ReadOnly[Literal["video"]]
    # ast-grep-ignore: no-long-names
    video: ReadOnly[TwelveLabsMarengo3TimedMediaInput]


class TwelveLabsMarengo3AudioRequest(TwelveLabsMarengo3RequestBase):
    inputType: ReadOnly[Literal["audio"]]
    # ast-grep-ignore: no-long-names
    audio: ReadOnly[TwelveLabsMarengo3TimedMediaInput]


# ast-grep-ignore: no-long-names
class TwelveLabsMarengo3TextImageRequest(TwelveLabsMarengo3RequestBase):
    inputType: ReadOnly[Literal["text_image"]]
    # ast-grep-ignore: no-long-names
    text_image: ReadOnly[TwelveLabsMarengo3TextImageInput]


# ast-grep-ignore: no-long-names
class TwelveLabsMarengo3MultiInputRequest(TwelveLabsMarengo3RequestBase):
    inputType: ReadOnly[Literal["multi_input"]]
    multi_input: ReadOnly[TwelveLabsMarengo3MultiInput]


# ast-grep-ignore: no-long-names
TwelveLabsMarengo3EmbeddingRequest: TypeAlias = (
    TwelveLabsMarengo3TextRequest
    | TwelveLabsMarengo3ImageRequest
    | TwelveLabsMarengo3VideoRequest
    | TwelveLabsMarengo3AudioRequest
    # ast-grep-ignore: no-long-names
    | TwelveLabsMarengo3TextImageRequest
    # ast-grep-ignore: no-long-names
    | TwelveLabsMarengo3MultiInputRequest
)


class TwelveLabsS3OutputDataConfig(TypedDict):
    s3Uri: str


class TwelveLabsOutputDataConfig(TypedDict):
    s3OutputDataConfig: TwelveLabsS3OutputDataConfig


class TwelveLabsAsyncInvokeRequest(TypedDict):
    modelId: str
    # ast-grep-ignore: no-long-names
    modelInput: ReadOnly[TwelveLabsMarengoEmbeddingRequest | TwelveLabsMarengo3EmbeddingRequest]
    outputDataConfig: TwelveLabsOutputDataConfig


# ast-grep-ignore: no-long-names
class TwelveLabsAsyncInvokeStatusResponse(TypedDict):
    invocationArn: str
    modelArn: str
    status: str
    submitTime: str
    lastModifiedTime: str
    endTime: str | None
    outputDataConfig: TwelveLabsOutputDataConfig
    clientRequestToken: str | None
    failureMessage: str | None
