from typing import Annotated, ClassVar, Final, Literal

from pydantic import ConfigDict, Field, JsonValue, field_validator

from betterlite.types.utils.base import ResponseObject

VALID_PIPELINE_ACTIONS: Final = {"allow", "block", "next", "modify_response"}
VALID_PIPELINE_MODES: Final = {"pre_call", "post_call"}


class PipelineStep(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    guardrail: str
    on_fail: str = "block"
    on_pass: str = Field(default="allow")
    on_error: str | None = None
    pass_data: bool = False
    modify_response_message: str | None = None

    @field_validator("on_fail", "on_pass", "on_error")
    @classmethod
    def validate_action(cls, value: str | None) -> str | None:
        if value is None:
            return None
        if value not in VALID_PIPELINE_ACTIONS:
            actions = sorted(VALID_PIPELINE_ACTIONS)
            raise ValueError(f"Invalid action '{value}'. Must be one of: {actions}")
        return value


class GuardrailPipeline(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    mode: str
    steps: Annotated[list[PipelineStep], Field(min_length=1)]

    @field_validator("mode")
    @classmethod
    def validate_mode(cls, value: str) -> str:
        if value not in VALID_PIPELINE_MODES:
            modes = sorted(VALID_PIPELINE_MODES)
            raise ValueError(f"Invalid mode '{value}'. Must be one of: {modes}")
        return value


class PipelineStepResult(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    guardrail_name: str
    outcome: Literal["pass", "fail", "error"]
    action_taken: str
    modified_data: dict[str, JsonValue] | None = None
    error_detail: str | None = None
    duration_seconds: float | None = None


class PipelineExecutionResult(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(
        extra="ignore", arbitrary_types_allowed=True,
    )

    terminal_action: str
    step_results: list[PipelineStepResult]
    modified_data: dict[str, JsonValue] | None = None
    error_message: str | None = None
    modify_response_message: str | None = None
    original_exception: Exception | None = Field(default=None, exclude=True)
