import httpx
from litellm.llms.azure.search.transformation import BingGroundingSearchConfig
from pydantic import JsonValue

from .inputs import (
    CapScenario,
    HeaderInput,
    HeaderScenario,
    MaxResultsInput,
    ReplyInput,
    ReplyScenario,
    RequestInput,
    RequestScenario,
    SettingsInput,
    UrlScenario,
)
from .surface import (
    ADAPTER,
    JSON,
    PROVIDER_ERROR,
    ErrorRecord,
    HiddenCarrier,
    Minter,
    apply_environment,
    logging_object,
)


def request_outcome(scenario: RequestScenario) -> JsonValue:
    apply_environment(scenario["environment"])
    config = ADAPTER.validate_python(BingGroundingSearchConfig(entra_token_minter=None))
    params = scenario.get("optional_params")
    try:
        body = config.transform_search_request(scenario["query"], {} if params is None else params)
    except ValueError:
        return {"ok": False, "status": None}
    return {"ok": True, "body": body}


def header_outcome(scenario: HeaderScenario) -> JsonValue:
    apply_environment(scenario["environment"])
    minter = Minter(scenario["minter"])
    config = ADAPTER.validate_python(BingGroundingSearchConfig(entra_token_minter=minter))
    api_base = scenario.get("api_base")
    try:
        headers = config.validate_environment(
            dict(scenario["headers"]), scenario.get("api_key"), api_base
        )
    except ValueError:
        return {"ok": False, "status": None, "calls": minter.calls}
    return {"ok": True, "headers": JSON.validate_python(headers), "calls": minter.calls}


def reply_outcome(scenario: ReplyScenario) -> JsonValue:
    apply_environment(scenario["environment"])
    config = ADAPTER.validate_python(BingGroundingSearchConfig(entra_token_minter=None))
    status = scenario.get("status")
    headers = scenario.get("headers")
    params = scenario.get("optional_params")
    request = httpx.Request("POST", "https://example.invalid/openai/v1/responses")
    raw_response = httpx.Response(
        200 if status is None else status,
        headers={} if headers is None else headers,
        stream=httpx.ByteStream(scenario["body"].encode()),
        request=request,
    )
    # a streamed response has no content until it is read
    raw_response.read()
    try:
        parsed = config.transform_search_response(
            raw_response, logging_object(), optional_params={} if params is None else params
        )
    except PROVIDER_ERROR as error:
        record = ErrorRecord.model_validate(error, from_attributes=True)
        return {"ok": False, "status": record.status_code}
    except ValueError:
        return {"ok": False, "status": None}
    hidden = HiddenCarrier.model_validate(parsed, from_attributes=True).params
    body = JSON.validate_json(parsed.model_dump_json())
    return {"ok": True, "body": body, "hidden": hidden}


def url_outcome(scenario: UrlScenario) -> JsonValue:
    apply_environment(scenario["environment"])
    config = ADAPTER.validate_python(BingGroundingSearchConfig(entra_token_minter=None))
    try:
        url = config.get_complete_url(scenario.get("api_base"), {})
    except ValueError:
        return {"ok": False, "status": None}
    return {"ok": True, "url": url}


def pair_outcome(scenario: CapScenario) -> JsonValue:
    params = scenario.get("optional_params")
    request: RequestScenario = {
        "environment": scenario["environment"],
        "query": scenario["query"],
    }
    reply: ReplyScenario = {
        "environment": scenario["environment"],
        "status": scenario.get("status"),
        "headers": {},
        "body": scenario["body"],
    }
    if params is not None:
        request["optional_params"] = params
        reply["optional_params"] = params
    return {"request": request_outcome(request), "reply": reply_outcome(reply)}


def request_report(input_value: RequestInput) -> JsonValue:
    outcomes: list[JsonValue] = [
        request_outcome(scenario) for scenario in input_value["scenarios"]
    ]
    return {"requests": outcomes}


def header_report(input_value: HeaderInput) -> JsonValue:
    outcomes: list[JsonValue] = [
        header_outcome(scenario) for scenario in input_value["scenarios"]
    ]
    return {"headers": outcomes}


def reply_report(input_value: ReplyInput) -> JsonValue:
    outcomes: list[JsonValue] = [
        reply_outcome(scenario) for scenario in input_value["scenarios"]
    ]
    return {"replies": outcomes}


def pair_report(input_value: MaxResultsInput) -> JsonValue:
    outcomes: list[JsonValue] = [
        pair_outcome(scenario) for scenario in input_value["scenarios"]
    ]
    return {"scenarios": outcomes}


def settings_report(input_value: SettingsInput) -> JsonValue:
    urls: list[JsonValue] = [url_outcome(scenario) for scenario in input_value["urls"]]
    requests: list[JsonValue] = [
        request_outcome(scenario) for scenario in input_value["requests"]
    ]
    return {"urls": urls, "requests": requests}
