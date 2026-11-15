from pydantic import JsonValue
from typing_extensions import TypedDict

from betterlite.types.guardrails.pii import PiiEntityType


class PresidioAnalyzeRequest(TypedDict, total=False):
    text: str
    language: str | None
    ad_hoc_recognizers: list[str] | None
    entities: list[PiiEntityType | str] | None


class PresidioAnalyzeResponseItem(TypedDict, total=False):
    entity_type: PiiEntityType | str | None
    start: int | None
    end: int | None
    score: float | None
    analysis_explanation: dict[str, JsonValue] | None
    recognition_metadata: dict[str, JsonValue] | None
