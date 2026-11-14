from collections.abc import Mapping, Sequence
from typing import Annotated, ClassVar, Final, Literal, Required, Self, TypeAlias

from pydantic import ConfigDict, Field, JsonValue, PrivateAttr, model_validator, with_config
from typing_extensions import ReadOnly, TypedDict

from betterlite.types.llms.base import LiteLLMPydanticObjectBase

DecisionsJSON: TypeAlias = str | Mapping[str, JsonValue] | Sequence[JsonValue]
NoulCriteria: TypeAlias = Mapping[Literal["true", "false"], DecisionsJSON | None]
MAX_DECISION_QUESTIONS: Final = 128

class NoulQuestion(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", frozen=True)
    type: Literal["noul"]
    instructions: DecisionsJSON | None = None
    criteria: NoulCriteria | None = None

    @model_validator(mode="after")
    def require_instructions(self) -> Self:
        if self.instructions is None and self.criteria is None:
            raise ValueError("A noul question requires instructions or criteria")
        return self

class ChoiceQuestion(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", frozen=True)
    type: Literal["choice"]
    instructions: DecisionsJSON | None = None
    criteria: Annotated[
        Mapping[str, DecisionsJSON | None], Field(min_length=1, max_length=255)
    ]

class ScoreQuestion(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", frozen=True)
    type: Literal["score"]
    instructions: DecisionsJSON | None = None
    criteria: Annotated[Sequence[DecisionsJSON], Field(min_length=1, max_length=10)]

DecisionQuestion: TypeAlias = Annotated[
    NoulQuestion | ChoiceQuestion | ScoreQuestion,
    Field(discriminator="type"),
]

DecisionQuestionMap: TypeAlias = Annotated[
    Mapping[Annotated[str, Field(min_length=1)], DecisionQuestion],
    Field(min_length=1, max_length=MAX_DECISION_QUESTIONS),
]

class DecisionsRequestBody(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", frozen=True)
    state: DecisionsJSON
    questions: DecisionQuestionMap

class DecisionsRequest(DecisionsRequestBody):
    model: str

@with_config(ConfigDict(extra="allow"))
class DecisionsCallParams(TypedDict, total=False):
    model: Required[ReadOnly[str]]
    state: Required[ReadOnly[DecisionsJSON]]
    questions: Required[ReadOnly[DecisionQuestionMap]]
    api_key: ReadOnly[str | None]
    api_base: ReadOnly[str | None]
    timeout: ReadOnly[float | None]
    custom_llm_provider: ReadOnly[str | None]
    extra_headers: ReadOnly[Mapping[str, str] | None]

class NoulAnswer(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", frozen=True)
    type: Literal["noul"]
    noul: float

class ChoiceAnswer(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", frozen=True)
    type: Literal["choice"]
    choice: str
    confidence: float
    probabilities: Mapping[str, float]

class ScoreAnswer(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", frozen=True)
    type: Literal["score"]
    score: float
    confidence: float
    legend: Mapping[str, DecisionsJSON]
    probabilities: Mapping[str, float]

DecisionAnswer: TypeAlias = Annotated[
    NoulAnswer | ChoiceAnswer | ScoreAnswer,
    Field(discriminator="type"),
]

class DecisionsUsage(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", frozen=True)
    input_tokens: int = 0
    output_tokens: int = 0
    cached_tokens: int = Field(default=0, exclude=True)
    cache_write_tokens: int = Field(default=0, exclude=True)

class _HiddenParamsResponse(LiteLLMPydanticObjectBase):
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @property
    def hidden_params(self) -> dict[str, JsonValue]:
        return self._hidden_params

    def set_hidden_params(self, params: Mapping[str, JsonValue]) -> None:
        self._hidden_params.update(params)

class DecisionsResponse(_HiddenParamsResponse):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", frozen=True)
    model: str | None = None
    answers: Mapping[str, DecisionAnswer]
    usage: DecisionsUsage | None = None

class OpenAIDecisionInputText(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", frozen=True)
    type: Literal["input_text"]
    text: str

class OpenAIDecisionInputImage(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", frozen=True)
    type: Literal["input_image"]
    image_url: str
    detail: str | None = None

OpenAIDecisionContentPart: TypeAlias = Annotated[
    OpenAIDecisionInputText | OpenAIDecisionInputImage,
    Field(discriminator="type"),
]

class OpenAIDecisionInputMessage(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", frozen=True)
    role: Literal["user"] = "user"
    type: Literal["message"] = "message"
    content: str | Sequence[OpenAIDecisionContentPart]

OpenAIDecisionInput: TypeAlias = str | Sequence[OpenAIDecisionInputMessage]

class OpenAIPredicateQuestion(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", frozen=True)
    type: Literal["predicate"]
    name: str | None = None
    instructions: str

class OpenAIChoiceOption(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", frozen=True)
    value: str | bool
    description: str | None = None

def systemone_choice_key(*, value: str | bool) -> str:
    if value is True:
        return "true"
    if value is False:
        return "false"
    return value

class OpenAIChoiceQuestion(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", frozen=True)
    type: Literal["choice"]
    name: str | None = None
    instructions: str
    choices: Annotated[Sequence[OpenAIChoiceOption], Field(min_length=2, max_length=255)]

    @model_validator(mode="after")
    def require_unique_systemone_keys(self) -> Self:
        keys: Final = frozenset(
            systemone_choice_key(value=option.value) for option in self.choices
        )
        if len(keys) != len(self.choices):
            raise ValueError(
                "Choice values must be unique, and a boolean cannot share its text with a "
                "string choice",
            )
        return self

class OpenAIScoreLevel(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", frozen=True)
    label: str
    description: str | None = None

class OpenAIScoreQuestion(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", frozen=True)
    type: Literal["score"]
    name: str | None = None
    instructions: str
    levels: Annotated[Sequence[OpenAIScoreLevel], Field(min_length=2, max_length=10)]

OpenAIDecisionQuestion: TypeAlias = Annotated[
    OpenAIPredicateQuestion | OpenAIChoiceQuestion | OpenAIScoreQuestion,
    Field(discriminator="type"),
]

class OpenAIDecisionRequestBody(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", frozen=True)
    input: OpenAIDecisionInput
    questions: Annotated[
        Sequence[OpenAIDecisionQuestion],
        Field(min_length=1, max_length=MAX_DECISION_QUESTIONS),
    ]
    safety_identifier: str | None = None

class OpenAIDecisionRequest(OpenAIDecisionRequestBody):
    model: str

class OpenAIPredicateAnswer(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    type: Literal["predicate"] = "predicate"
    name: str | None
    probability: float

class OpenAIChoiceProbability(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    value: str | bool
    probability: float

class OpenAIChoiceAnswer(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    type: Literal["choice"] = "choice"
    name: str | None
    choice: str | bool
    probabilities: tuple[OpenAIChoiceProbability, ...]
    confidence: float

class OpenAIScoreProbability(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    value: int
    label: str
    probability: float

class OpenAIScoreAnswer(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    type: Literal["score"] = "score"
    name: str | None
    score: float
    probabilities: tuple[OpenAIScoreProbability, ...]
    confidence: float

class OpenAIRefusalAnswer(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    type: Literal["refusal"] = "refusal"
    name: str | None

OpenAIDecisionAnswer: TypeAlias = Annotated[
    OpenAIPredicateAnswer | OpenAIChoiceAnswer | OpenAIScoreAnswer | OpenAIRefusalAnswer,
    Field(discriminator="type"),
]

class OpenAIDecisionInputTokensDetails(LiteLLMPydanticObjectBase):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    cached_tokens: int = 0
    cache_write_tokens: int = 0

# ast-grep-ignore: no-long-names
class OpenAIDecisionOutputTokensDetails(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    reasoning_tokens: int = 0

class OpenAIDecisionUsage(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)
    input_tokens: int
    # ast-grep-ignore: no-long-names
    input_tokens_details: OpenAIDecisionInputTokensDetails = OpenAIDecisionInputTokensDetails()
    output_tokens: int
    # ast-grep-ignore: no-long-names
    output_tokens_details: OpenAIDecisionOutputTokensDetails = OpenAIDecisionOutputTokensDetails()
    total_tokens: int

class OpenAIDecisionResponse(_HiddenParamsResponse):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", frozen=True)
    model: str
    answers: tuple[OpenAIDecisionAnswer, ...]
    usage: OpenAIDecisionUsage
