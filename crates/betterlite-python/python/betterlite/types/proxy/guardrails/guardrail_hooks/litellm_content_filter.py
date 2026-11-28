from enum import Enum
from typing import Literal, TypeAlias

from pydantic import BaseModel, JsonValue
from typing_extensions import TypedDict, override

from betterlite.types.llms.base import BaseLiteLLMOpenAIResponseObject
from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel

CompetitorIntentType: TypeAlias = Literal[
    "competitor_comparison",
    "possible_competitor_comparison",
    "category_ranking",
    "log_only",
    "other",
]
CompetitorActionHint: TypeAlias = Literal["allow", "reframe", "refuse", "escalate", "log_only"]


class CompetitorIntentEvidenceEntry(TypedDict, total=False):
    type: Literal["entity", "signal"]
    key: str
    value: str | None
    match: str


class CompetitorIntentResult(TypedDict, total=False):
    intent: CompetitorIntentType
    confidence: float
    entities: dict[str, list[str]]
    signals: list[str]
    action_hint: CompetitorActionHint
    evidence: list[CompetitorIntentEvidenceEntry]


class DetectionType(str, Enum):
    PATTERN = "pattern"
    BLOCKED_WORD = "blocked_word"
    CATEGORY_KEYWORD = "category_keyword"


class PatternDetection(TypedDict):
    type: Literal["pattern"]
    pattern_name: str
    action: str


class BlockedWordDetection(TypedDict):
    type: Literal["blocked_word"]
    keyword: str
    action: str
    description: str | None


class CategoryKeywordDetection(TypedDict):
    type: Literal["category_keyword"]
    category: str
    keyword: str
    severity: str
    action: str


class CompetitorIntentDetection(TypedDict):
    type: Literal["competitor_intent"]
    intent: str
    confidence: float
    action_hint: str
    entities: dict[str, list[str]]
    signals: list[str]
    evidence: list[dict[str, JsonValue]]


ContentFilterDetection: TypeAlias = (
    PatternDetection
    | BlockedWordDetection
    | CategoryKeywordDetection
    | CompetitorIntentDetection
)


# ast-grep-ignore: no-long-names
class ContentFilterCategoryConfig(BaseLiteLLMOpenAIResponseObject):
    category: str
    enabled: bool = True
    action: Literal["BLOCK", "MASK"]
    severity_threshold: Literal["high", "medium", "low"] = "medium"
    category_file: str | None = None


# ast-grep-ignore: no-long-names
class LitellmContentFilterGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    patterns: list[dict[str, JsonValue]] | None = None
    blocked_words: list[dict[str, JsonValue]] | None = None
    blocked_words_file: str | None = None
    categories: list[ContentFilterCategoryConfig] | None = None
    severity_threshold: str = "medium"
    pattern_redaction_format: str | None = "[{pattern_name}_REDACTED]"
    keyword_redaction_tag: str | None = "[KEYWORD_REDACTED]"
    competitor_intent_config: dict[str, JsonValue] | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "LiteLLM Content Filter"
