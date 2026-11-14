from typing import Literal, TypeAlias, TypedDict

BedrockGuardrailQualifier: TypeAlias = Literal[
    "grounding_source", "query", "guard_content"
]


class BedrockTextContent(TypedDict, total=False):
    text: str
    qualifiers: list[BedrockGuardrailQualifier]


class BedrockContentItem(TypedDict, total=False):
    text: BedrockTextContent


class BedrockRequest(TypedDict, total=False):
    source: Literal["INPUT", "OUTPUT"]
    content: list[BedrockContentItem]


BedrockGuardrailUsage = TypedDict(
    "BedrockGuardrailUsage",
    {
        "topicPolicyUnits": int | None,
        "contentPolicyUnits": int | None,
        "wordPolicyUnits": int | None,
        "sensitiveInformationPolicyUnits": int | None,
        "sensitiveInformationPolicyFreeUnits": int | None,
        "contextualGroundingPolicyUnits": int | None,
        "contentPolicyImageUnits": int | None,
        "automatedReasoningPolicyUnits": int | None,
        "automatedReasoningPolicies": int | None,
    },
    total=False,
)


class BedrockGuardrailOutput(TypedDict, total=False):
    text: str | None


# ast-grep-ignore: no-long-names
class BedrockGuardrailTopicPolicyItem(TypedDict, total=False):
    name: str | None
    type: str | None
    action: str | None


class BedrockGuardrailTopicPolicy(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    topics: list[BedrockGuardrailTopicPolicyItem]


# ast-grep-ignore: no-long-names
BedrockGuardrailContentPolicyFilter = TypedDict(
    "BedrockGuardrailContentPolicyFilter",
    {
        "type": str | None,
        "confidence": str | None,
        "filterStrength": str | None,
        "action": str | None,
    },
    total=False,
)


class BedrockGuardrailContentPolicy(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    filters: list[BedrockGuardrailContentPolicyFilter]


# ast-grep-ignore: no-long-names
class BedrockGuardrailWordPolicyCustomWord(TypedDict, total=False):
    match: str
    action: str


# ast-grep-ignore: no-long-names
class BedrockGuardrailWordPolicyManagedWord(TypedDict, total=False):
    match: str | None
    type: str | None
    action: str | None


BedrockGuardrailWordPolicy = TypedDict(
    "BedrockGuardrailWordPolicy",
    {
        # ast-grep-ignore: no-long-names
        "customWords": list[BedrockGuardrailWordPolicyCustomWord],
        # ast-grep-ignore: no-long-names
        "managedWordLists": list[BedrockGuardrailWordPolicyManagedWord],
    },
    total=False,
)


class BedrockGuardrailPiiEntity(TypedDict, total=False):
    type: str | None
    match: str | None
    action: str | None


class BedrockGuardrailRegex(TypedDict, total=False):
    name: str | None
    regex: str | None
    match: str | None
    action: str | None


# ast-grep-ignore: no-long-names
BedrockGuardrailSensitiveInformationPolicy = TypedDict(
    "BedrockGuardrailSensitiveInformationPolicy",
    {
        "piiEntities": list[BedrockGuardrailPiiEntity] | None,
        "regexes": list[BedrockGuardrailRegex] | None,
    },
    total=False,
)


# ast-grep-ignore: no-long-names
class BedrockGuardrailContextualGroundingFilter(TypedDict, total=False):
    type: str | None
    threshold: float | None
    score: float | None
    action: str | None


# ast-grep-ignore: no-long-names
class BedrockGuardrailContextualGroundingPolicy(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    filters: list[BedrockGuardrailContextualGroundingFilter]


BedrockGuardrailCoverage = TypedDict(
    "BedrockGuardrailCoverage",
    {
        "textCharacters": dict[str, int],
    },
    total=False,
)


# ast-grep-ignore: no-long-names
BedrockGuardrailInvocationMetrics = TypedDict(
    "BedrockGuardrailInvocationMetrics",
    {
        "guardrailProcessingLatency": int,
        "usage": BedrockGuardrailUsage,
        "guardrailCoverage": BedrockGuardrailCoverage,
    },
    total=False,
)


BedrockGuardrailAssessment = TypedDict(
    "BedrockGuardrailAssessment",
    {
        "topicPolicy": BedrockGuardrailTopicPolicy | None,
        "contentPolicy": BedrockGuardrailContentPolicy | None,
        "wordPolicy": BedrockGuardrailWordPolicy | None,
        # ast-grep-ignore: no-long-names
        "sensitiveInformationPolicy": BedrockGuardrailSensitiveInformationPolicy | None,
        # ast-grep-ignore: no-long-names
        "contextualGroundingPolicy": BedrockGuardrailContextualGroundingPolicy | None,
        # ast-grep-ignore: no-long-names
        "invocationMetrics": BedrockGuardrailInvocationMetrics,
        "guardrailCoverage": BedrockGuardrailCoverage,
    },
    total=False,
)


class BedrockGuardrailResponse(TypedDict, total=False):
    usage: BedrockGuardrailUsage | None
    action: str | None
    output: list[BedrockGuardrailOutput] | None
    outputs: list[BedrockGuardrailOutput] | None
    assessments: list[BedrockGuardrailAssessment] | None


class BedrockChecksTextContent(TypedDict, total=False):
    text: str


class BedrockChecksMessage(TypedDict, total=False):
    role: Literal["user", "assistant", "system"]
    content: list[BedrockChecksTextContent]


BedrockChecksScoreEntry = TypedDict(
    "BedrockChecksScoreEntry",
    {
        "category": str | None,
        "severityScore": float | None,
    },
    total=False,
)


BedrockChecksPiiEntry = TypedDict(
    "BedrockChecksPiiEntry",
    {
        "type": str | None,
        "confidenceScore": float | None,
        "messageIndex": int | None,
        "contentIndex": int | None,
        "beginOffset": int | None,
        "endOffset": int | None,
    },
    total=False,
)


class BedrockChecksScoreResult(TypedDict, total=False):
    results: list[BedrockChecksScoreEntry]


# ast-grep-ignore: no-long-names
class BedrockChecksSensitiveInformationResult(TypedDict, total=False):
    results: list[BedrockChecksPiiEntry]
    truncated: bool | None


BedrockChecksResults = TypedDict(
    "BedrockChecksResults",
    {
        "contentFilter": BedrockChecksScoreResult | None,
        "promptAttack": BedrockChecksScoreResult | None,
        # ast-grep-ignore: no-long-names
        "sensitiveInformation": BedrockChecksSensitiveInformationResult | None,
    },
    total=False,
)


BedrockChecksViolation = TypedDict(
    "BedrockChecksViolation",
    {
        "check": str,
        "category": str | None,
        "type": str | None,
        "severityScore": float,
        "confidenceScore": float,
        "truncated": bool,
    },
    total=False,
)


BedrockChecksTextUnits = TypedDict(
    "BedrockChecksTextUnits",
    {
        "textUnits": int | None,
    },
    total=False,
)


BedrockChecksUsage = TypedDict(
    "BedrockChecksUsage",
    {
        "contentFilter": BedrockChecksTextUnits | None,
        "promptAttack": BedrockChecksTextUnits | None,
        "sensitiveInformation": BedrockChecksTextUnits | None,
    },
    total=False,
)


class BedrockGuardrailChecksResponse(TypedDict, total=False):
    results: BedrockChecksResults | None
    usage: BedrockChecksUsage | None
