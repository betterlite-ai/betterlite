from typing import ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator
from typing_extensions import override

from betterlite.types.decisions import MAX_DECISION_QUESTIONS
from betterlite.types.llms.base import LiteLLMPydanticObjectBase
from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class DecisionModelCheck(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    name: str = Field(min_length=1)
    instructions: str = Field(min_length=1)
    threshold: float = Field(default=0.7, ge=0.0, le=1.0)
    action: Literal["block", "log"] = "block"

    @field_validator("instructions")
    @classmethod
    def _instructions_not_blank(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("instructions must not be blank")
        return value


# ast-grep-ignore: no-long-names
class DecisionModelGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    decision_model: str
    checks: tuple[DecisionModelCheck, ...] = Field(
        min_length=1,
        max_length=MAX_DECISION_QUESTIONS,
    )
    max_input_chars: int = Field(default=24000, gt=0)
    max_concurrent_decision_calls: int = Field(default=8, gt=0)
    streaming_end_of_stream_only: bool = False
    streaming_sampling_rate: int = Field(default=5, ge=1)

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Decision Model"
