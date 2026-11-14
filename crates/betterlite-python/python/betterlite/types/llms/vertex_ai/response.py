from typing import Literal, Required

from pydantic import JsonValue
from typing_extensions import TypedDict

from betterlite.types.llms.vertex_ai.content import HttpxContentType
from betterlite.types.llms.vertex_ai.safety import (
    HarmCategory,
    HarmProbability,
    HarmSeverity,
)
from betterlite.types.llms.vertex_ai.usage import UsageMetadata


class SafetyRatings(TypedDict):
    category: HarmCategory
    probability: HarmProbability
    probabilityScore: int
    severity: HarmSeverity
    blocked: bool


class Date(TypedDict):
    year: int
    month: int
    date: int


class Citation(TypedDict):
    startIndex: int
    endIndex: int
    uri: str
    title: str
    license: str
    publicationDate: Date


class CitationMetadata(TypedDict):
    citations: list[Citation]


class SearchEntryPoint(TypedDict, total=False):
    renderedContent: str
    sdkBlob: str


class GroundingMetadata(TypedDict, total=False):
    webSearchQueries: list[str]
    searchEntryPoint: SearchEntryPoint
    groundingAttributions: list[dict[str, JsonValue]]


class LogprobsCandidate(TypedDict):
    token: str
    tokenId: int
    logProbability: float


class LogprobsTopCandidate(TypedDict):
    candidates: list[LogprobsCandidate]


class LogprobsResult(TypedDict, total=False):
    topCandidates: list[LogprobsTopCandidate]
    chosenCandidates: list[LogprobsCandidate]


class UrlMetadata(TypedDict, total=False):
    retrievedUrl: str
    urlRetrievalStatus: str


class UrlContextMetadata(TypedDict, total=False):
    urlMetadata: list[UrlMetadata]


GeminiFinishReason = Literal[
    "FINISH_REASON_UNSPECIFIED",
    "STOP",
    "MAX_TOKENS",
    "SAFETY",
    "RECITATION",
    "LANGUAGE",
    "OTHER",
    "BLOCKLIST",
    "PROHIBITED_CONTENT",
    "SPII",
    "MALFORMED_FUNCTION_CALL",
    "IMAGE_SAFETY",
    "IMAGE_PROHIBITED_CONTENT",
    "TOO_MANY_TOOL_CALLS",
    "MALFORMED_RESPONSE",
    "NO_IMAGE",
    "IMAGE_RECITATION",
    "IMAGE_OTHER",
    "ESCALATION",
    "UNEXPECTED_TOOL_CALL",
    "MISSING_THOUGHT_SIGNATURE",
]


class Candidates(TypedDict, total=False):
    index: int
    content: HttpxContentType
    finishReason: GeminiFinishReason
    safetyRatings: list[SafetyRatings]
    citationMetadata: CitationMetadata
    groundingMetadata: GroundingMetadata
    finishMessage: str
    logprobsResult: LogprobsResult
    urlContextMetadata: UrlContextMetadata


class PromptFeedback(TypedDict):
    blockReason: str
    safetyRatings: list[SafetyRatings]
    blockReasonMessage: str


class GenerateContentResponseBody(TypedDict, total=False):
    candidates: list[Candidates]
    promptFeedback: PromptFeedback
    usageMetadata: Required[UsageMetadata]
    responseId: str
