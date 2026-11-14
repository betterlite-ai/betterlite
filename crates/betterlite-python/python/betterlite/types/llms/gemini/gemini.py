from enum import Enum
from typing import ClassVar, Literal, Required

from pydantic import ConfigDict, JsonValue
from typing_extensions import TypedDict

from betterlite.types.llms.gemini.vertex_ai_types import (
    GenerationConfig,
    HttpxBlobType,
    HttpxContentType,
    Tools,
    UsageMetadata,
)
from betterlite.types.utils.base import ResponseObject


class GeminiFilesState(Enum):
    STATE_UNSPECIFIED = "STATE_UNSPECIFIED"
    PROCESSING = "PROCESSING"
    ACTIVE = "ACTIVE"
    FAILED = "FAILED"


class GeminiFilesSource(Enum):
    SOURCE_UNSPECIFIED = "SOURCE_UNSPECIFIED"
    UPLOADED = "UPLOADED"
    GENERATED = "GENERATED"


class GeminiCreateFilesResponseObject(TypedDict):  # ast-grep-ignore: no-long-names
    name: str
    displayName: str
    mimeType: str
    sizeBytes: str
    createTime: str
    updateTime: str
    expirationTime: str
    sha256Hash: str
    uri: str
    state: GeminiFilesState
    source: GeminiFilesSource
    error: dict[str, JsonValue]
    metadata: dict[str, JsonValue]


class BidiGenerateContentTranscription(TypedDict):  # ast-grep-ignore: no-long-names
    text: str


class BidiGenerateContentServerContent(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    generationComplete: bool
    turnComplete: bool
    interrupted: bool
    groundingMetadata: dict[str, JsonValue]
    inputTranscription: BidiGenerateContentTranscription  # ast-grep-ignore: no-long-names
    outputTranscription: BidiGenerateContentTranscription  # ast-grep-ignore: no-long-names
    modelTurn: HttpxContentType


class BidiGenerateContentServerMessage(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    usageMetadata: UsageMetadata
    serverContent: BidiGenerateContentServerContent  # ast-grep-ignore: no-long-names
    setupComplete: dict[str, JsonValue]


class BidiGenerateContentRealtimeInput(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    text: str
    audio: HttpxBlobType
    video: HttpxBlobType
    audioStreamEnd: bool
    activityStart: bool
    activityEnd: bool


StartOfSpeechSensitivityEnum = Literal[
    "START_SENSITIVITY_UNSPECIFIED", "START_SENSITIVITY_HIGH", "START_SENSITIVITY_LOW"
]
EndOfSpeechSensitivityEnum = Literal[
    "END_SENSITIVITY_UNSPECIFIED", "END_SENSITIVITY_HIGH", "END_SENSITIVITY_LOW"
]


class AutomaticActivityDetection(TypedDict, total=False):
    disabled: bool
    startOfSpeechSensitivity: StartOfSpeechSensitivityEnum
    prefixPaddingMs: int
    endOfSpeechSensitivity: EndOfSpeechSensitivityEnum
    silenceDurationMs: int


# ast-grep-ignore: no-long-names
class BidiGenerateContentRealtimeInputConfig(TypedDict, total=False):
    automaticActivityDetection: AutomaticActivityDetection


class BidiGenerateContentSetup(TypedDict, total=False):
    model: str
    generationConfig: GenerationConfig
    systemInstruction: HttpxContentType
    tools: list[Tools]
    realtimeInputConfig: BidiGenerateContentRealtimeInputConfig  # ast-grep-ignore: no-long-names
    sessionResumption: dict[str, JsonValue]
    sessionResumptionConfig: dict[str, JsonValue]
    contextWindowCompression: dict[str, JsonValue]
    inputAudioTranscription: dict[str, JsonValue]
    outputAudioTranscription: dict[str, JsonValue]


class GeminiImageGenerationInstance(TypedDict):
    prompt: str


class GeminiImageGenerationParameters(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    sampleCount: int | None = None  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L166
    aspectRatio: str | None = None  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L169
    imageSize: str | None = None  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L172
    personGeneration: str | None = None  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L175
    background: str | None = None
    input_fidelity: str | None = None
    moderation: str | None = None
    output_compression: str | None = None
    output_format: str | None = None
    quality: str | None = None
    response_format: str | None = None
    style: str | None = None
    user: str | None = None


class GeminiImageGenerationRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    instances: list[GeminiImageGenerationInstance]
    parameters: GeminiImageGenerationParameters  # ast-grep-ignore: no-long-names


class GeminiGeneratedImage(TypedDict):
    bytesBase64Encoded: str


class GeminiImageGenerationPrediction(TypedDict):  # ast-grep-ignore: no-long-names
    generatedImages: list[GeminiGeneratedImage]


class GeminiImageGenerationResponse(TypedDict):
    predictions: list[GeminiImageGenerationPrediction]  # ast-grep-ignore: no-long-names


class GeminiVideoGenerationInstance(TypedDict, total=False):
    prompt: Required[str]
    image: dict[str, JsonValue]


class GeminiVideoGenerationParameters(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    aspectRatio: str | None = None  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L248
    durationSeconds: int | None = None  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L251
    resolution: str | None = None
    negativePrompt: str | None = None  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L263
    lastFrame: JsonValue = None  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L266
    referenceImages: list[JsonValue] | None = None  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L272
    video: JsonValue = None
    personGeneration: str | None = None  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L284


class GeminiVideoGenerationRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    instances: list[GeminiVideoGenerationInstance]
    parameters: GeminiVideoGenerationParameters | None = None  # ast-grep-ignore: no-long-names


class GeminiVideoUri(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    uri: str


class GeminiGeneratedVideoSample(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    video: GeminiVideoUri


class GeminiGenerateVideoResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    generatedSamples: list[GeminiGeneratedVideoSample]  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L318


class GeminiOperationResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    generateVideoResponse: GeminiGenerateVideoResponse  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L325


class GeminiOperationMetadata(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    createTime: str | None = None  # noqa: N815  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/gemini.py#L332
    model: str | None = None


class GeminiLongRunningOperationResponse(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    name: str
    done: bool = False
    metadata: GeminiOperationMetadata | None = None
    response: GeminiOperationResponse | None = None
    error: dict[str, JsonValue] | None = None
