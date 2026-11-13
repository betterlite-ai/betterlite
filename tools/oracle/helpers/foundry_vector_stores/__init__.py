from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue

from foundry_vector_stores.documents import citation_report, create_report
from foundry_vector_stores.inputs import (
    AuthCredentialsInput,
    AzureSearchEndpointInput,
    EndpointScenario,
    HeaderScenario,
    Input,
)
from foundry_vector_stores.search import embedded_report, search_report
from foundry_vector_stores.surface import CONFIG, JSON, SETTINGS


def endpoints_report() -> JsonValue:
    endpoints = CONFIG.get_vector_store_endpoints_by_type()
    read: list[list[str]] = [[method, path] for method, path in endpoints["read"]]
    write: list[list[str]] = [[method, path] for method, path in endpoints["write"]]
    pairs: dict[str, list[list[str]]] = {"read": read, "write": write}
    return {"endpoints": JSON.validate_python(pairs)}


def credential_outcome(params: dict[str, JsonValue]) -> dict[str, JsonValue]:
    try:
        resolved = SETTINGS.get_auth_credentials(litellm_params=params)
    except ValueError:
        return {"ok": False}
    return {"ok": True, "headers": JSON.validate_python(resolved.get("headers"))}


def header_outcome(scenario: HeaderScenario) -> dict[str, JsonValue]:
    params = GenericLiteLLMParams(api_key=scenario["api_key"])
    headers = SETTINGS.validate_environment(
        headers=dict(scenario["headers"]), litellm_params=params
    )
    return {"ok": True, "headers": JSON.validate_python(headers)}


def credentials_report(input_value: AuthCredentialsInput) -> JsonValue:
    credentials: list[JsonValue] = [
        credential_outcome(params) for params in input_value["credentials"]
    ]
    environments: list[JsonValue] = [
        header_outcome(scenario) for scenario in input_value["environments"]
    ]
    return {"credentials": credentials, "environments": environments}


def endpoint_outcome(scenario: EndpointScenario) -> dict[str, JsonValue]:
    try:
        url = SETTINGS.get_complete_url(scenario["api_base"], scenario["litellm_params"])
    except ValueError:
        return {"ok": False, "status": None}
    return {"ok": True, "url": url}


def endpoint_report(input_value: AzureSearchEndpointInput) -> JsonValue:
    scenarios: list[JsonValue] = [
        endpoint_outcome(scenario) for scenario in input_value["scenarios"]
    ]
    return {"scenarios": scenarios}


def run(input_value: Input) -> JsonValue:
    if "store_type_endpoints" in input_value:
        return endpoints_report()
    if "auth_credentials" in input_value:
        return credentials_report(input_value["auth_credentials"])
    if "search_query_fields" in input_value:
        return search_report(input_value["search_query_fields"])
    if "vector_query_with_embedding" in input_value:
        return embedded_report(input_value["vector_query_with_embedding"])
    if "document_score_citation" in input_value:
        return citation_report(input_value["document_score_citation"])
    if "unsupported_store_create" in input_value:
        return create_report()
    if "azure_search_endpoint" in input_value:
        return endpoint_report(input_value["azure_search_endpoint"])
    raise ValueError("expected one foundry_vector_stores variant")
