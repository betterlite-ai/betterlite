from pydantic import JsonValue
from typing_extensions import NotRequired, TypedDict


class OptionalParams(TypedDict, total=False):
    filters: dict[str, JsonValue] | None
    max_num_results: int | None
    ranking_options: dict[str, JsonValue] | None
    rewrite_query: bool | None
    top_k: int


class SearchScenario(TypedDict):
    vector_store_id: str
    query: str | list[str]
    api_base: str
    litellm_params: dict[str, JsonValue]
    optional_params: NotRequired[OptionalParams]
    extra_body: NotRequired[dict[str, JsonValue] | None]
    embedding: NotRequired[list[float] | None]
    asynchronous: NotRequired[bool]


class SearchQueryFieldsInput(TypedDict):
    searches: list[SearchScenario]


class VectorQueryWithEmbeddingInput(TypedDict):
    scenarios: list[SearchScenario]


class HeaderScenario(TypedDict):
    headers: dict[str, str]
    api_key: str | None


class AuthCredentialsInput(TypedDict):
    credentials: list[dict[str, JsonValue]]
    environments: list[HeaderScenario]


class CitationScenario(TypedDict):
    status: int
    headers: list[tuple[str, str]]
    body: str
    before_search: SearchScenario | None


class DocumentScoreCitationInput(TypedDict):
    responses: list[CitationScenario]


class EndpointScenario(TypedDict):
    api_base: str | None
    litellm_params: dict[str, JsonValue]


class AzureSearchEndpointInput(TypedDict):
    scenarios: list[EndpointScenario]


class EmptyInput(TypedDict):
    pass


class Input(TypedDict, total=False):
    store_type_endpoints: EmptyInput
    auth_credentials: AuthCredentialsInput
    search_query_fields: SearchQueryFieldsInput
    vector_query_with_embedding: VectorQueryWithEmbeddingInput
    document_score_citation: DocumentScoreCitationInput
    unsupported_store_create: EmptyInput
    azure_search_endpoint: AzureSearchEndpointInput
