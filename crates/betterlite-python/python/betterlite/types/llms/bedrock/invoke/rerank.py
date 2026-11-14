from typing import Literal, Required

from pydantic import JsonValue
from typing_extensions import TypedDict


class BedrockRerankTextQuery(TypedDict):
    text: str


class BedrockRerankQuery(TypedDict):
    textQuery: BedrockRerankTextQuery
    type: Literal["TEXT"]


class BedrockRerankModelConfiguration(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    modelArn: Required[str]
    modelConfiguration: dict[str, JsonValue]


# ast-grep-ignore: no-long-names
class BedrockRerankBedrockRerankingConfiguration(TypedDict):
    # ast-grep-ignore: no-long-names
    modelConfiguration: BedrockRerankModelConfiguration
    numberOfResults: int


class BedrockRerankConfiguration(TypedDict):
    # ast-grep-ignore: no-long-names
    bedrockRerankingConfiguration: BedrockRerankBedrockRerankingConfiguration
    type: Literal["BEDROCK_RERANKING_MODEL"]


class BedrockRerankTextDocument(TypedDict, total=False):
    text: str


class BedrockRerankInlineDocumentSource(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    jsonDocument: dict[str, JsonValue]
    textDocument: BedrockRerankTextDocument
    type: Literal["TEXT", "JSON"]


class BedrockRerankSource(TypedDict):
    # ast-grep-ignore: no-long-names
    inlineDocumentSource: BedrockRerankInlineDocumentSource
    type: Literal["INLINE"]


class BedrockRerankRequest(TypedDict):
    queries: list[BedrockRerankQuery]
    rerankingConfiguration: BedrockRerankConfiguration
    sources: list[BedrockRerankSource]
