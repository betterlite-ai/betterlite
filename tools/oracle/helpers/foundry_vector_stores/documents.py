import httpx
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.router import GenericLiteLLMParams
from litellm.types.vector_stores import VectorStoreCreateOptionalRequestParams
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import TypedDict

from foundry_vector_stores.inputs import CitationScenario, DocumentScoreCitationInput
from foundry_vector_stores.search import request_outcome
from foundry_vector_stores.surface import CONFIG, DETAILS, SETTINGS, logging_fixture


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


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int | None = None


PAGE: TypeAdapter[SearchPage] = TypeAdapter(SearchPage)


def citation_outcome(scenario: CitationScenario) -> dict[str, JsonValue]:
    logging = logging_fixture()
    before = scenario["before_search"]
    if before is not None and request_outcome(before, logging)["ok"] is False:
        return {"ok": False, "status": None}
    response = httpx.Response(
        scenario["status"], content=scenario["body"].encode(), headers=scenario["headers"]
    )
    try:
        reply = SETTINGS.transform_search_vector_store_response(
            response=response, litellm_logging_obj=logging
        )
    except Exception as error:
        failure = Failure.model_validate(error)
        if failure.status_code is None:
            raise
        return {"ok": False, "status": failure.status_code}
    page = DETAILS.validate_json(PAGE.dump_json(PAGE.validate_python(reply)))
    return {"ok": True, **page}


def citation_report(input_value: DocumentScoreCitationInput) -> JsonValue:
    responses: list[JsonValue] = [
        citation_outcome(scenario) for scenario in input_value["responses"]
    ]
    return {"responses": responses}


def fixture_response(request: httpx.Request) -> httpx.Response:
    body = {"object": "list"}
    return httpx.Response(200, json=body, request=request)


def create_report() -> JsonValue:
    transport = httpx.MockTransport(fixture_response)
    client = HTTPHandler(client=httpx.Client(transport=transport))
    params = GenericLiteLLMParams(api_key="fixture-key", api_base="https://fixture.example")
    request: dict[str, JsonValue] = {"ok": True}
    try:
        BaseLLMHTTPHandler().vector_store_create_handler(
            vector_store_create_optional_params=VectorStoreCreateOptionalRequestParams(),
            vector_store_provider_config=CONFIG,
            custom_llm_provider="azure_ai",
            litellm_params=params,
            logging_obj=logging_fixture(),
            client=client,
        )
    except NotImplementedError:
        request = {"ok": False, "status": None}
    reply: dict[str, JsonValue] = {"ok": True}
    try:
        CONFIG.transform_create_vector_store_response(response=httpx.Response(200))
    except NotImplementedError:
        reply = {"ok": False, "status": None}
    return {"request": request, "response": reply}
