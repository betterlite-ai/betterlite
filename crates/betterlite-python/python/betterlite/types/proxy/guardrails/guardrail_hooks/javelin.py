from pydantic import BaseModel, JsonValue
from typing_extensions import TypedDict, override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel


class JavelinGuardInput(TypedDict):
    text: str


class JavelinGuardRequest(TypedDict):
    input: JavelinGuardInput
    config: dict[str, JsonValue] | None
    metadata: dict[str, JsonValue] | None


# ast-grep-ignore: no-long-names
class JavelinPromptInjectionCategories(TypedDict):
    prompt_injection: bool
    jailbreak: bool


# ast-grep-ignore: no-long-names
class JavelinPromptInjectionCategoryScores(TypedDict):
    prompt_injection: float
    jailbreak: float


class JavelinPromptInjectionResults(TypedDict):
    # ast-grep-ignore: no-long-names
    categories: JavelinPromptInjectionCategories
    # ast-grep-ignore: no-long-names
    category_scores: JavelinPromptInjectionCategoryScores
    reject_prompt: str


# ast-grep-ignore: no-long-names
class JavelinPromptInjectionAssessment(TypedDict):
    results: JavelinPromptInjectionResults
    request_reject: bool


class JavelinTrustSafetyCategories(TypedDict):
    violence: bool
    weapons: bool
    hate_speech: bool
    crime: bool
    sexual: bool
    profanity: bool


# ast-grep-ignore: no-long-names
class JavelinTrustSafetyCategoryScores(TypedDict):
    violence: float
    weapons: float
    hate_speech: float
    crime: float
    sexual: float
    profanity: float


class JavelinTrustSafetyResults(TypedDict):
    categories: JavelinTrustSafetyCategories
    # ast-grep-ignore: no-long-names
    category_scores: JavelinTrustSafetyCategoryScores


class JavelinTrustSafetyAssessment(TypedDict):
    results: JavelinTrustSafetyResults
    request_reject: bool


# ast-grep-ignore: no-long-names
class JavelinLanguageDetectionResults(TypedDict):
    lang: str
    prob: float


# ast-grep-ignore: no-long-names
class JavelinLanguageDetectionAssessment(TypedDict):
    # ast-grep-ignore: no-long-names
    results: JavelinLanguageDetectionResults
    request_reject: bool


class JavelinGuardResponse(TypedDict):
    assessments: list[
        dict[
            str,
            # ast-grep-ignore: no-long-names
            JavelinPromptInjectionAssessment
            | JavelinTrustSafetyAssessment
            # ast-grep-ignore: no-long-names
            | JavelinLanguageDetectionAssessment,
        ]
    ]


class JavelinGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    guard_name: str | None = None
    api_version: str | None = "v1"
    metadata: dict[str, JsonValue] | None = None
    application: str | None = None
    config: dict[str, JsonValue] | None = None

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "Javelin Guardrails"
