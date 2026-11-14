import math
from typing import ClassVar, Literal

from pydantic import BaseModel, ConfigDict, StrictFloat, StrictInt, field_validator
from typing_extensions import ReadOnly, TypedDict


class ROIEstimatorChanges(BaseModel):
    additions: int
    deletions: int
    files: int
    commits: int


class ROIEstimatorFile(BaseModel):
    filename: str | None
    status: str | None
    additions: int | None
    deletions: int | None


class ROIEstimatorCommit(BaseModel):
    sha: str
    message: str
    additions: int | None = None
    deletions: int | None = None
    changed_files: int | None = None


class ROIEstimatorEvidence(BaseModel):
    repo: str
    number: int
    title: str
    body: str
    changes: ROIEstimatorChanges
    files: tuple[ROIEstimatorFile, ...]
    commits: tuple[ROIEstimatorCommit, ...]


class ROICompletionMessage(TypedDict):
    role: ReadOnly[Literal["system", "user"]]
    content: ReadOnly[str]


class ROICompletionMetadata(TypedDict):
    tags: ReadOnly[tuple[str, ...]]
    litellm_roi_estimator: ReadOnly[bool]


class ROIResponseFormat(TypedDict):
    type: ReadOnly[Literal["json_object"]]


class ROICompletionRequest(BaseModel):
    model: str
    temperature: Literal[0]
    messages: tuple[ROICompletionMessage, ...]
    response_format: ROIResponseFormat
    max_tokens: Literal[1200]
    metadata: ROICompletionMetadata
    reasoning_effort: Literal["none"] | None = None


class _ROICompletionMessageResponse(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(from_attributes=True)

    content: str | None = None


class _ROICompletionChoice(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(from_attributes=True)

    finish_reason: str | None = None
    message: _ROICompletionMessageResponse


class ROICompletionResponse(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(from_attributes=True)

    choices: tuple[_ROICompletionChoice, ...]


class ROIEstimatorResult(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(strict=True, extra="forbid")

    hours: StrictInt | StrictFloat
    reasoning: str

    @field_validator("hours")
    @classmethod
    def validate_hours(cls, value: StrictInt | StrictFloat) -> StrictInt | StrictFloat:
        if not math.isfinite(value) or value < 0:
            raise ValueError("Hours must be finite and nonnegative.")
        return value

    @field_validator("reasoning")
    @classmethod
    def validate_reasoning(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("Reasoning must not be empty.")
        return value
