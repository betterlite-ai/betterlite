from typing import Literal

from typing_extensions import TypedDict


class PromptTokensDetails(TypedDict):
    modality: Literal["TEXT", "AUDIO", "IMAGE", "VIDEO"]
    tokenCount: int


class UsageMetadata(TypedDict, total=False):
    promptTokenCount: int
    totalTokenCount: int
    candidatesTokenCount: int
    responseTokenCount: int
    cachedContentTokenCount: int
    toolUsePromptTokenCount: int
    toolUsePromptTokensDetails: list[PromptTokensDetails]
    promptTokensDetails: list[PromptTokensDetails]
    cacheTokensDetails: list[PromptTokensDetails]
    thoughtsTokenCount: int
    responseTokensDetails: list[PromptTokensDetails]
    candidatesTokensDetails: list[PromptTokensDetails]


class TokenCountDetailsResponse(TypedDict):
    totalTokens: int
    promptTokensDetails: list[PromptTokensDetails]
