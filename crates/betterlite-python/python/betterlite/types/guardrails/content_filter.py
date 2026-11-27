from enum import Enum
from typing import Final, Literal

from betterlite.types.proxy.guardrails.guardrail_hooks.litellm_content_filter import (
    ContentFilterCategoryConfig,
)
from betterlite.types.utils.base import ResponseObject


class ContentFilterAction(str, Enum):
    BLOCK = "BLOCK"
    MASK = "MASK"


class BlockedWord(ResponseObject):
    keyword: str
    action: ContentFilterAction
    description: str | None = None


class ContentFilterPattern(ResponseObject):
    pattern_type: Literal["prebuilt", "regex"]
    pattern_name: str | None = None
    pattern: str | None = None
    name: str | None = None
    action: ContentFilterAction


class ContentFilterConfigModel(ResponseObject):
    patterns: list[ContentFilterPattern] | None = None
    blocked_words: list[BlockedWord] | None = None
    blocked_words_file: str | None = None
    categories: list[ContentFilterCategoryConfig] | None = None
    severity_threshold: str | None = None
    pattern_redaction_format: str | None = None
    keyword_redaction_tag: str | None = None


MCP_SECURITY_ON_VIOLATION: Final = frozenset({"block", "alert"})
