import asyncio
import datetime as dt
from collections.abc import Mapping
from typing import Protocol, TypedDict, TypeVar

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.vector_stores.transformation import AzureAIVectorStoreConfig
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import EmbeddingResponse
from litellm.types.vector_stores import (
    VectorStoreCreateOptionalRequestParams,
    VectorStoreSearchResponse,
)
from pydantic import JsonValue, TypeAdapter

litellm.enable_azure_ad_token_refresh = False
CONFIG = AzureAIVectorStoreConfig()
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
DETAILS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
T = TypeVar("T")


class AuthHeaders(TypedDict, total=False):
    headers: dict[str, str]


class SettingsConfig(Protocol):
    def get_auth_credentials(self, litellm_params: dict[str, JsonValue]) -> AuthHeaders: ...

    def validate_environment(
        self, headers: dict[str, str], litellm_params: GenericLiteLLMParams
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

SETTINGS: SettingsConfig = CONFIG


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
    optional_params: OptionalParams
    litellm_params: dict[str, JsonValue]
    extra_body: dict[str, JsonValue] | None
    embedding: list[float] | None


class SearchQueryFieldsInput(TypedDict):
    searches: list[SearchScenario]


class EmbeddedScenario(TypedDict):
    vector_store_id: str
    query: str | list[str]
    api_base: str
    optional_params: OptionalParams
    litellm_params: dict[str, JsonValue]
    embedding: list[float]
    asynchronous: bool


class VectorQueryWithEmbeddingInput(TypedDict):
    scenarios: list[EmbeddedScenario]


class HeaderScenario(TypedDict):
    headers: dict[str, str]
    api_key: str | None


class AuthCredentialsInput(TypedDict):
    credentials: list[dict[str, JsonValue]]
    environments: list[HeaderScenario]


class BeforeSearch(TypedDict):
    vector_store_id: str
    query: str | list[str]
    api_base: str
    litellm_params: dict[str, JsonValue]


class CitationScenario(TypedDict):
    status: int
    headers: list[tuple[str, str]]
    body: str
    before_search: BeforeSearch | None


class DocumentScoreCitationInput(TypedDict):
    responses: list[CitationScenario]


class EndpointScenario(TypedDict):
    api_base: str | None
    litellm_params: dict[str, JsonValue]


class AzureSearchEndpointInput(TypedDict):
    scenarios: list[EndpointScenario]


class EmptyInput(TypedDict):
    pass


class SearchContent(TypedDict):
    text: str | None
    type: str | None


class SearchRow(TypedDict):
    score: float | None
    content: list[SearchContent] | None
    file_id: str | None
    filename: str | None
    attributes: JsonValue


class SearchPage(TypedDict):
    object: str
    search_query: str | None
    data: list[SearchRow] | None


PAGE: TypeAdapter[SearchPage] = TypeAdapter(SearchPage)


class Input(TypedDict, total=False):
    store_type_endpoints: EmptyInput
    auth_credentials: AuthCredentialsInput
    search_query_fields: SearchQueryFieldsInput
    vector_query_with_embedding: VectorQueryWithEmbeddingInput
    document_score_citation: DocumentScoreCitationInput
    unsupported_store_create: EmptyInput
    azure_search_endpoint: AzureSearchEndpointInput


class FixtureExecutor:
    def __init__(self, vectors: list[list[float]]) -> None:
        self.vectors = vectors
        self.calls: list[JsonValue] = []

    def record(self, model: str, query: str, configuration: Mapping[str, T]) -> None:
        recorded = DETAILS.validate_python(dict(configuration))
        call = {"model": model, "query": query, "configuration": recorded}
        self.calls.append(JSON.validate_python(call))

    def response(self) -> EmbeddingResponse:
        index = len(self.calls) - 1
        vector: list[float] = []
        if index < len(self.vectors):
            vector = self.vectors[index]
        elif self.vectors:
            vector = self.vectors[-1]
        return EmbeddingResponse(
            model="fixture-model",
            data=[{"object": "embedding", "embedding": vector, "index": 0}],
        )

    def embed(self, model: str, query: str, configuration: Mapping[str, T]) -> EmbeddingResponse:
        self.record(model, query, configuration)
        return self.response()

    async def aembed(
        self, model: str, query: str, configuration: Mapping[str, T]
    ) -> EmbeddingResponse:
        self.record(model, query, configuration)
        await asyncio.sleep(0)
        return self.response()


def logging_fixture() -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type="vector_store_search",
        start_time=dt.datetime.now(dt.UTC),
        litellm_call_id="foundry-vector-stores",
        function_id="foundry-vector-stores",
    )


def logging_report(logging: Logging) -> JsonValue:
    details = DETAILS.validate_python(logging.model_call_details)
    return JSON.validate_python(
        {
            "input": details.get("input"),
            "embedding_model": details.get("embedding_model"),
            "top_k": details.get("top_k"),
        }
    )


def store_type_endpoints_report(_input: EmptyInput) -> JsonValue:
    endpoints = CONFIG.get_vector_store_endpoints_by_type()
    return JSON.validate_python(
        {
            "endpoints": {
                "read": [[method, path] for method, path in endpoints["read"]],
                "write": [[method, path] for method, path in endpoints["write"]],
            }
        }
    )


def credential_outcome(params: dict[str, JsonValue]) -> JsonValue:
    try:
        resolved = SETTINGS.get_auth_credentials(litellm_params=params)
    except ValueError:
        return JSON.validate_python({"ok": False})
    return JSON.validate_python({"ok": True, "headers": resolved.get("headers")})


def header_report(scenario: HeaderScenario) -> JsonValue:
    params = GenericLiteLLMParams(api_key=scenario["api_key"])
    headers = SETTINGS.validate_environment(
        headers=dict(scenario["headers"]), litellm_params=params
    )
    return JSON.validate_python({"ok": True, "headers": headers})


def credentials_report(input_value: AuthCredentialsInput) -> JsonValue:
    credentials = [credential_outcome(params) for params in input_value["credentials"]]
    environments = [header_report(scenario) for scenario in input_value["environments"]]
    return JSON.validate_python({"credentials": credentials, "environments": environments})


def search_outcome(scenario: SearchScenario) -> JsonValue:
    vectors: list[float] = [0.25, 0.5]
    if scenario["embedding"] is not None:
        vectors = scenario["embedding"]
    executor = FixtureExecutor([vectors])
    logging = logging_fixture()
    try:
        url, body = CONFIG.transform_search_vector_store_request(
            vector_store_id=scenario["vector_store_id"],
            query=scenario["query"],
            vector_store_search_optional_params=scenario["optional_params"],
            api_base=scenario["api_base"],
            litellm_logging_obj=logging,
            litellm_params=scenario["litellm_params"],
            extra_body=scenario["extra_body"],
            embedding_executor=executor,
        )
    except (ValueError, IndexError, KeyError):
        return JSON.validate_python(
            {
                "ok": False,
                "status": None,
                "url": None,
                "body": None,
                "calls": executor.calls,
                "extra_body": scenario["extra_body"],
            }
        )
    return JSON.validate_python(
        {
            "ok": True,
            "url": url,
            "body": body,
            "logging": logging_report(logging),
            "calls": executor.calls,
            "extra_body": scenario["extra_body"],
        }
    )


def search_report(input_value: SearchQueryFieldsInput) -> JsonValue:
    searches = [search_outcome(scenario) for scenario in input_value["searches"]]
    return JSON.validate_python({"searches": searches})


def embedded_outcome(scenario: EmbeddedScenario) -> JsonValue:
    executor = FixtureExecutor([scenario["embedding"]])
    logging = logging_fixture()
    try:
        if scenario["asynchronous"]:
            url, body = asyncio.run(
                CONFIG.atransform_search_vector_store_request(
                    vector_store_id=scenario["vector_store_id"],
                    query=scenario["query"],
                    vector_store_search_optional_params=scenario["optional_params"],
                    api_base=scenario["api_base"],
                    litellm_logging_obj=logging,
                    litellm_params=scenario["litellm_params"],
                    extra_body=None,
                    embedding_executor=executor,
                )
            )
        else:
            url, body = CONFIG.transform_search_vector_store_request(
                vector_store_id=scenario["vector_store_id"],
                query=scenario["query"],
                vector_store_search_optional_params=scenario["optional_params"],
                api_base=scenario["api_base"],
                litellm_logging_obj=logging,
                litellm_params=scenario["litellm_params"],
                extra_body=None,
                embedding_executor=executor,
            )
    except (ValueError, IndexError, KeyError):
        return JSON.validate_python(
            {"ok": False, "status": None, "url": None, "body": None, "calls": executor.calls}
        )
    return JSON.validate_python(
        {"ok": True, "url": url, "body": body, "calls": executor.calls}
    )


def embedded_report(input_value: VectorQueryWithEmbeddingInput) -> JsonValue:
    scenarios = [embedded_outcome(scenario) for scenario in input_value["scenarios"]]
    return JSON.validate_python({"scenarios": scenarios})


def run_before_search(before: BeforeSearch, logging: Logging) -> bool:
    executor = FixtureExecutor([[0.25, 0.5]])
    try:
        CONFIG.transform_search_vector_store_request(
            vector_store_id=before["vector_store_id"],
            query=before["query"],
            vector_store_search_optional_params=OptionalParams(),
            api_base=before["api_base"],
            litellm_logging_obj=logging,
            litellm_params=before["litellm_params"],
            extra_body=None,
            embedding_executor=executor,
        )
    except (ValueError, IndexError, KeyError):
        return False
    return True


def result_report(result: SearchRow) -> JsonValue:
    content = result["content"]
    return JSON.validate_python(
        {
            "score": result["score"],
            "content": None
            if content is None
            else [{"text": item["text"], "type": item["type"]} for item in content],
            "file_id": result["file_id"],
            "filename": result["filename"],
            "attributes": result["attributes"],
        }
    )


def page_report(page: VectorStoreSearchResponse) -> JsonValue:
    loaded = PAGE.validate_python(page)
    data = loaded["data"]
    return JSON.validate_python(
        {
            "ok": True,
            "object": loaded["object"],
            "search_query": loaded["search_query"],
            "data": None if data is None else [result_report(result) for result in data],
        }
    )


def citation_outcome(scenario: CitationScenario) -> JsonValue:
    logging = logging_fixture()
    before = scenario["before_search"]
    if before is not None:
        prepared = run_before_search(before, logging)
        if not prepared:
            return JSON.validate_python({"ok": False, "status": None})
    response = httpx.Response(
        scenario["status"], content=scenario["body"].encode(), headers=scenario["headers"]
    )
    try:
        page = CONFIG.transform_search_vector_store_response(
            response=response, litellm_logging_obj=logging
        )
    except BaseLLMException as error:
        return JSON.validate_python({"ok": False, "status": error.status_code})
    return page_report(page)


def citation_report(input_value: DocumentScoreCitationInput) -> JsonValue:
    responses = [citation_outcome(scenario) for scenario in input_value["responses"]]
    return JSON.validate_python({"responses": responses})


def fixture_response(request: httpx.Request) -> httpx.Response:
    body = {"object": "list"}
    return httpx.Response(200, json=body, request=request)


def mock_handler() -> HTTPHandler:
    transport = httpx.MockTransport(fixture_response)
    return HTTPHandler(client=httpx.Client(transport=transport))


def create_request_outcome() -> JsonValue:
    client = mock_handler()
    params = GenericLiteLLMParams(api_key="fixture-key", api_base="https://fixture.example")
    logging = logging_fixture()
    try:
        BaseLLMHTTPHandler().vector_store_create_handler(
            vector_store_create_optional_params=VectorStoreCreateOptionalRequestParams(),
            vector_store_provider_config=CONFIG,
            custom_llm_provider="azure_ai",
            litellm_params=params,
            logging_obj=logging,
            client=client,
        )
    except NotImplementedError:
        return JSON.validate_python({"ok": False, "status": None})
    return JSON.validate_python({"ok": True})


def create_response_outcome() -> JsonValue:
    try:
        CONFIG.transform_create_vector_store_response(response=httpx.Response(200))
    except NotImplementedError:
        return JSON.validate_python({"ok": False, "status": None})
    return JSON.validate_python({"ok": True})


def create_report(_input: EmptyInput) -> JsonValue:
    return JSON.validate_python(
        {"request": create_request_outcome(), "response": create_response_outcome()}
    )


def endpoint_outcome(scenario: EndpointScenario) -> JsonValue:
    try:
        url = SETTINGS.get_complete_url(
            api_base=scenario["api_base"], litellm_params=scenario["litellm_params"]
        )
    except ValueError:
        return JSON.validate_python({"ok": False, "status": None})
    return JSON.validate_python({"ok": True, "url": url})


def endpoint_report(input_value: AzureSearchEndpointInput) -> JsonValue:
    scenarios = [endpoint_outcome(scenario) for scenario in input_value["scenarios"]]
    return JSON.validate_python({"scenarios": scenarios})


def remaining_reports(input_value: Input) -> JsonValue:
    if "document_score_citation" in input_value:
        return citation_report(input_value["document_score_citation"])
    if "unsupported_store_create" in input_value:
        return create_report(input_value["unsupported_store_create"])
    if "azure_search_endpoint" in input_value:
        return endpoint_report(input_value["azure_search_endpoint"])
    raise ValueError("expected one foundry_vector_stores variant")


def run(input_value: Input) -> JsonValue:
    if "store_type_endpoints" in input_value:
        return store_type_endpoints_report(input_value["store_type_endpoints"])
    if "auth_credentials" in input_value:
        return credentials_report(input_value["auth_credentials"])
    if "search_query_fields" in input_value:
        return search_report(input_value["search_query_fields"])
    if "vector_query_with_embedding" in input_value:
        return embedded_report(input_value["vector_query_with_embedding"])
    return remaining_reports(input_value)
