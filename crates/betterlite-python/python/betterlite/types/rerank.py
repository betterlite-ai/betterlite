from typing import ClassVar, Literal

from pydantic import ConfigDict, JsonValue, PrivateAttr
from typing_extensions import ReadOnly, Required, TypedDict

from betterlite.types.utils.base import ResponseObject


class RerankRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    model: str
    query: str
    top_n: int | None = None
    documents: list[str | dict[str, JsonValue]]
    rank_fields: list[str] | None = None
    return_documents: bool | None = None
    max_chunks_per_doc: int | None = None
    max_tokens_per_doc: int | None = None
    instruction: str | None = None
    truncate_prompt_tokens: int | None = None
    truncation_side: Literal["left", "right"] | None = None
    max_tokens_per_query: int | None = None


class HostedVLLMRerankTruncationParams(ResponseObject):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="ignore")

    truncate_prompt_tokens: int | None = None
    truncation_side: Literal["left", "right"] | None = None
    max_tokens_per_query: int | None = None
    max_tokens_per_doc: int | None = None


class OptionalRerankParams(TypedDict, total=False):
    query: str
    top_n: int | None
    documents: list[str | dict[str, JsonValue]]
    rank_fields: list[str] | None
    return_documents: bool | None
    max_chunks_per_doc: int | None
    max_tokens_per_doc: int | None
    instruction: str | None
    truncate_prompt_tokens: ReadOnly[int | None]
    truncation_side: ReadOnly[Literal["left", "right"] | None]
    max_tokens_per_query: ReadOnly[int | None]


class RerankBilledUnits(TypedDict, total=False):
    search_units: int | None
    total_tokens: int | None


class RerankTokens(TypedDict, total=False):
    input_tokens: int | None
    output_tokens: int | None


class RerankResponseMeta(TypedDict, total=False):
    api_version: dict[str, JsonValue] | None
    billed_units: RerankBilledUnits | None
    tokens: RerankTokens | None


class RerankResponseDocument(TypedDict):
    text: str


class RerankResponseResult(TypedDict, total=False):
    index: Required[int]
    relevance_score: Required[float]
    document: RerankResponseDocument


class RerankResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    id: str | None = None
    results: list[RerankResponseResult] | None = None
    meta: RerankResponseMeta | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)
