from typing import Annotated, ClassVar, Literal, TypedDict

from pydantic import ConfigDict, Field

from betterlite.types.utils.base import ResponseObject


class VertexSpeechToTextAutoDecodingConfig(TypedDict):  # ast-grep-ignore: no-long-names
    pass


class VertexSpeechToTextRecognitionFeatures(TypedDict):  # ast-grep-ignore: no-long-names
    enableAutomaticPunctuation: bool


class VertexSpeechToTextRecognitionConfig(TypedDict):  # ast-grep-ignore: no-long-names
    model: str
    languageCodes: list[str]
    features: VertexSpeechToTextRecognitionFeatures  # ast-grep-ignore: no-long-names
    autoDecodingConfig: VertexSpeechToTextAutoDecodingConfig  # ast-grep-ignore: no-long-names


class VertexSpeechToTextRecognizeRequest(TypedDict):  # ast-grep-ignore: no-long-names
    config: VertexSpeechToTextRecognitionConfig  # ast-grep-ignore: no-long-names
    content: str


class VertexSpeechToTextAlternative(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    transcript: str | None = None


class VertexSpeechToTextResult(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    alternatives: list[VertexSpeechToTextAlternative] = Field(default_factory=list)
    languageCode: str | None = None


class VertexSpeechToTextResponseMetadata(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    totalBilledDuration: str | None = None


class VertexSpeechToTextRecognizeResponse(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    results: list[VertexSpeechToTextResult] = Field(default_factory=list)
    metadata: VertexSpeechToTextResponseMetadata | None = None  # ast-grep-ignore: no-long-names


class VertexSpeechStreamingConfigure(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="ignore")

    kind: Literal["configure"] = "configure"
    model: str
    language_codes: tuple[str, ...]
    sample_rate_hertz: int


class VertexSpeechStreamingFinishTurn(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="ignore")

    kind: Literal["finish_turn"] = "finish_turn"


class VertexSpeechStreamingDiscardTurn(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="ignore")

    kind: Literal["discard_turn"] = "discard_turn"


VertexSpeechStreamingCommandUnion = (  # ast-grep-ignore: no-long-names
    VertexSpeechStreamingConfigure
    # ast-grep-ignore: no-long-names
    | VertexSpeechStreamingFinishTurn
    # ast-grep-ignore: no-long-names
    | VertexSpeechStreamingDiscardTurn
)
VertexSpeechStreamingCommand = Annotated[
    VertexSpeechStreamingCommandUnion,  # ast-grep-ignore: no-long-names
    Field(discriminator="kind"),
]


class VertexSpeechStreamingResult(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="ignore")

    transcript: str
    is_final: bool


class VertexSpeechStreamingResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="ignore")

    kind: Literal["response"] = "response"
    speech_event: Literal["none", "begin", "end"]
    results: tuple[VertexSpeechStreamingResult, ...]
    billed_seconds: float


class VertexSpeechStreamingConfigured(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="ignore")

    kind: Literal["configured"] = "configured"


class VertexSpeechStreamingTurnFinished(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="ignore")

    kind: Literal["turn_finished"] = "turn_finished"


class VertexSpeechStreamingTurnDiscarded(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="ignore")

    kind: Literal["turn_discarded"] = "turn_discarded"
    billed_seconds: float


VertexSpeechStreamingEventUnion = (  # ast-grep-ignore: no-long-names
    VertexSpeechStreamingResponse
    # ast-grep-ignore: no-long-names
    | VertexSpeechStreamingConfigured
    # ast-grep-ignore: no-long-names
    | VertexSpeechStreamingTurnFinished
    # ast-grep-ignore: no-long-names
    | VertexSpeechStreamingTurnDiscarded
)
# ast-grep-ignore: no-long-names
VertexSpeechStreamingEvent = Annotated[VertexSpeechStreamingEventUnion, Field(discriminator="kind")]
