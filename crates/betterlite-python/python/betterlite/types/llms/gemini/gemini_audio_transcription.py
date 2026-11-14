from typing import ClassVar, Literal, Required, TypedDict

from pydantic import ConfigDict
from typing_extensions import ReadOnly

from betterlite.types.utils.base import ResponseObject


class GeminiTranscriptionAudioInput(TypedDict):
    type: ReadOnly[Literal["audio"]]
    data: ReadOnly[str]
    mime_type: ReadOnly[str]


class GeminiTranscriptionVerbatimMode(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    type: Required[ReadOnly[Literal["verbatim"]]]
    timestamp_granularities: ReadOnly[tuple[Literal["word"], ...]]
    diarization_mode: ReadOnly[Literal["speaker"]]


class GeminiTranscriptionConfig(TypedDict, total=False):
    language_codes: ReadOnly[tuple[str, ...]]
    mode: ReadOnly[GeminiTranscriptionVerbatimMode]  # ast-grep-ignore: no-long-names


class GeminiTranscriptionGenerationConfig(TypedDict):  # ast-grep-ignore: no-long-names
    transcription_config: ReadOnly[GeminiTranscriptionConfig]


# ast-grep-ignore: no-long-names
class GeminiTranscriptionInteractionRequest(TypedDict, total=False):
    model: Required[ReadOnly[str]]
    input: Required[ReadOnly[tuple[GeminiTranscriptionAudioInput, ...]]]
    # ast-grep-ignore: no-long-names
    generation_config: ReadOnly[GeminiTranscriptionGenerationConfig]


class GeminiTranscriptionWordAnnotation(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    type: str | None = None
    text: str | None = None
    speaker: str | None = None
    start_offset: str | None = None
    end_offset: str | None = None


class GeminiTranscriptionContent(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    type: str | None = None
    text: str | None = None
    # ast-grep-ignore: no-long-names
    annotations: tuple[GeminiTranscriptionWordAnnotation, ...] = ()


class GeminiTranscriptionStep(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    type: str | None = None
    content: tuple[GeminiTranscriptionContent, ...] = ()


class GeminiTranscriptionModalityTokens(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    modality: str | None = None
    tokens: int = 0


class GeminiTranscriptionUsage(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    total_tokens: int = 0
    total_input_tokens: int = 0
    total_output_tokens: int = 0
    # ast-grep-ignore: no-long-names
    input_tokens_by_modality: tuple[GeminiTranscriptionModalityTokens, ...] = ()


class GeminiTranscriptionInteractionResponse(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    id: str | None = None
    status: str | None = None
    usage: GeminiTranscriptionUsage | None = None
    steps: tuple[GeminiTranscriptionStep, ...] = ()
