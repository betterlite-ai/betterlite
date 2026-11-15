from typing import Final, Literal, TypeAlias

from pydantic import BaseModel, Field
from typing_extensions import TypedDict, override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel

CodeBlockActionTaken: TypeAlias = Literal["block", "allow", "log_only"]

BLOCKED_LANGUAGES_OPTIONS: Final = [
    "python",
    "javascript",
    "typescript",
    "bash",
    "ruby",
    "go",
    "java",
    "csharp",
    "php",
    "c",
    "cpp",
    "rust",
    "sql",
]


class CodeBlockDetection(TypedDict, total=False):
    type: Literal["code_block"]
    language: str
    confidence: float
    action_taken: CodeBlockActionTaken
    evidence: str | None
    snippet: str | None


# ast-grep-ignore: no-long-names
class BlockCodeExecutionGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    blocked_languages: list[str] | None = Field(
        default=None,
        json_schema_extra={
            "ui_type": "multiselect",
            "options": list(BLOCKED_LANGUAGES_OPTIONS),
        },
    )
    action: Literal["block", "mask"] = "block"
    confidence_threshold: float = Field(
        default=0.5,
        ge=0.0,
        le=1.0,
        json_schema_extra={
            "ui_type": "percentage",
            "min": 0.0,
            "max": 1.0,
            "step": 0.1,
            "default_value": 0.5,
        },
    )
    detect_execution_intent: bool = True

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Block Code Execution"
