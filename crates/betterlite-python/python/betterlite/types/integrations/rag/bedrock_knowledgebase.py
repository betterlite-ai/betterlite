from typing import Literal

from pydantic import JsonValue
from typing_extensions import ReadOnly, TypedDict


class BedrockKBLocation(TypedDict, total=False):
    type: str
    s3Location: dict[str, JsonValue] | None
    webLocation: dict[str, JsonValue] | None
    kendraDocumentLocation: dict[str, JsonValue] | None
    salesforceLocation: dict[str, JsonValue] | None
    sharePointLocation: dict[str, JsonValue] | None
    confluenceLocation: dict[str, JsonValue] | None
    customDocumentLocation: dict[str, JsonValue] | None
    sqlLocation: dict[str, JsonValue] | None


class BedrockKBRowValue(TypedDict):
    columnName: str
    columnValue: str
    type: str


class BedrockKBContent(TypedDict, total=False):
    type: str
    text: str | None
    byteContent: str | None
    row: list[BedrockKBRowValue] | None


class BedrockKBRetrievalResult(TypedDict, total=False):
    content: BedrockKBContent | None
    location: BedrockKBLocation | None
    score: float | None
    metadata: dict[str, JsonValue] | None


class BedrockKBResponse(TypedDict, total=False):
    guardrailAction: Literal["INTERVENED", "NONE"] | None
    nextToken: str | None
    retrievalResults: list[BedrockKBRetrievalResult] | None


class BedrockKBMetadataAttribute(TypedDict, total=False):
    description: str | None
    key: str | None
    type: str | None


# ast-grep-ignore: no-long-names
class BedrockKBImplicitFilterConfiguration(TypedDict, total=False):
    metadataAttributes: list[BedrockKBMetadataAttribute] | None
    modelArn: str | None


# ast-grep-ignore: no-long-names
class BedrockKBSelectiveModeConfiguration(TypedDict, total=False):
    pass


class BedrockKBMetadataConfiguration(TypedDict, total=False):
    selectionMode: str | None
    # ast-grep-ignore: no-long-names
    selectiveModeConfiguration: BedrockKBSelectiveModeConfiguration | None


class BedrockKBModelConfiguration(TypedDict, total=False):
    additionalModelRequestFields: dict[str, JsonValue] | None
    modelArn: str | None


# ast-grep-ignore: no-long-names
class BedrockKBRerankingConfiguration(TypedDict, total=False):
    bedrockRerankingConfiguration: dict[str, JsonValue] | None
    type: str | None


# ast-grep-ignore: no-long-names
class BedrockKBVectorSearchConfiguration(TypedDict, total=False):
    filter: dict[str, JsonValue] | None
    # ast-grep-ignore: no-long-names
    implicitFilterConfiguration: BedrockKBImplicitFilterConfiguration | None
    numberOfResults: int | None
    overrideSearchType: str | None
    # ast-grep-ignore: no-long-names
    rerankingConfiguration: BedrockKBRerankingConfiguration | None


# ast-grep-ignore: no-long-names
class BedrockKBRetrievalConfiguration(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    vectorSearchConfiguration: BedrockKBVectorSearchConfiguration | None


class BedrockKBRetrievalQuery(TypedDict, total=False):
    text: str | None


# ast-grep-ignore: no-long-names
class BedrockKBGuardrailConfiguration(TypedDict, total=False):
    guardrailId: str | None
    guardrailVersion: str | None


class BedrockKBUserContext(TypedDict):
    userId: ReadOnly[str]


class BedrockKBRequest(TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    guardrailConfiguration: BedrockKBGuardrailConfiguration | None
    nextToken: str | None
    # ast-grep-ignore: no-long-names
    retrievalConfiguration: BedrockKBRetrievalConfiguration | None
    retrievalQuery: BedrockKBRetrievalQuery
    userContext: ReadOnly[BedrockKBUserContext | None]
