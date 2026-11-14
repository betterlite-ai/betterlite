from typing import Literal

from typing_extensions import TypedDict

HarmCategory = Literal[
    "HARM_CATEGORY_UNSPECIFIED",
    "HARM_CATEGORY_HATE_SPEECH",
    "HARM_CATEGORY_DANGEROUS_CONTENT",
    "HARM_CATEGORY_HARASSMENT",
    "HARM_CATEGORY_SEXUALLY_EXPLICIT",
]
HarmBlockThreshold = Literal[
    "HARM_BLOCK_THRESHOLD_UNSPECIFIED",
    "BLOCK_LOW_AND_ABOVE",
    "BLOCK_MEDIUM_AND_ABOVE",
    "BLOCK_ONLY_HIGH",
    "BLOCK_NONE",
]
HarmBlockMethod = Literal["HARM_BLOCK_METHOD_UNSPECIFIED", "SEVERITY", "PROBABILITY"]
HarmProbability = Literal["HARM_PROBABILITY_UNSPECIFIED", "NEGLIGIBLE", "LOW", "MEDIUM", "HIGH"]
HarmSeverity = Literal[
    "HARM_SEVERITY_UNSPECIFIED",
    "HARM_SEVERITY_NEGLIGIBLE",
    "HARM_SEVERITY_LOW",
    "HARM_SEVERITY_MEDIUM",
    "HARM_SEVERITY_HIGH",
]


class SafetSettingsConfig(TypedDict, total=False):
    category: HarmCategory
    threshold: HarmBlockThreshold
    max_influential_terms: int
    method: HarmBlockMethod
