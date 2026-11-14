from typing import ClassVar, Literal, TypedDict

from pydantic import ConfigDict
from typing_extensions import ReadOnly

from betterlite.types.utils.base import ResponseObject


class VertexGeminiTranscriptionInlineData(TypedDict):  # ast-grep-ignore: no-long-names
    mimeType: ReadOnly[str]
    data: ReadOnly[str]


class VertexGeminiTranscriptionPart(TypedDict):
    inlineData: ReadOnly[VertexGeminiTranscriptionInlineData]  # ast-grep-ignore: no-long-names


class VertexGeminiTranscriptionContent(TypedDict):  # ast-grep-ignore: no-long-names
    role: ReadOnly[Literal["user"]]
    parts: ReadOnly[tuple[VertexGeminiTranscriptionPart, ...]]


# ast-grep-ignore: no-long-names
class VertexGeminiTranscriptionAudioConfig(TypedDict, total=False):
    languageCodes: ReadOnly[tuple[str, ...]]


# ast-grep-ignore: no-long-names
class VertexGeminiTranscriptionGenerationConfig(TypedDict):
    # ast-grep-ignore: no-long-names
    audioTranscriptionConfig: ReadOnly[VertexGeminiTranscriptionAudioConfig]


# ast-grep-ignore: no-long-names
class VertexGeminiTranscriptionRequest(TypedDict):
    # ast-grep-ignore: no-long-names
    contents: ReadOnly[tuple[VertexGeminiTranscriptionContent, ...]]
    # ast-grep-ignore: no-long-names
    generationConfig: ReadOnly[VertexGeminiTranscriptionGenerationConfig]


class VertexGeminiTranscriptionResponsePart(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    text: str | None = None


class VertexGeminiTranscriptionResponseContent(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    parts: tuple[VertexGeminiTranscriptionResponsePart, ...] = ()  # ast-grep-ignore: no-long-names


class VertexGeminiTranscriptionCandidate(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    # ast-grep-ignore: no-long-names
    content: VertexGeminiTranscriptionResponseContent | None = None


class VertexGeminiTranscriptionModalityTokens(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    modality: str | None = None
    tokenCount: int = 0


class VertexGeminiTranscriptionUsageMetadata(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    promptTokenCount: int = 0
    candidatesTokenCount: int = 0
    totalTokenCount: int = 0
    # ast-grep-ignore: no-long-names
    promptTokensDetails: tuple[VertexGeminiTranscriptionModalityTokens, ...] = ()


class VertexGeminiTranscriptionResponse(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    # ast-grep-ignore: no-long-names
    candidates: tuple[VertexGeminiTranscriptionCandidate, ...] = ()
    # ast-grep-ignore: no-long-names
    usageMetadata: VertexGeminiTranscriptionUsageMetadata | None = None
