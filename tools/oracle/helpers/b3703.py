from typing import Literal, TypeAlias, TypedDict

from litellm.proxy.hooks.prompt_injection_detection import (
    _OPTIONAL_PromptInjectionDetection,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/prompt_injection_detection.py#L37
)


class KeywordsCall(TypedDict):
    operation: Literal["keywords"]


class SimilarityCall(TypedDict):
    operation: Literal["similarity"]
    user_input: str
    threshold: str


Input: TypeAlias = KeywordsCall | SimilarityCall


def run(input_value: Input) -> list[str] | bool:
    detection = _OPTIONAL_PromptInjectionDetection(prompt_injection_params=None)
    if input_value["operation"] == "keywords":
        return detection.generate_injection_keywords()
    return detection.check_user_input_similarity(
        input_value["user_input"], float(input_value["threshold"])
    )
