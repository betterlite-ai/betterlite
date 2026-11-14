from litellm.llms.bedrock_mantle.responses.transformation import BedrockMantleResponsesAPIConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue

from .calls import config_of, environment, map_result, warnings_recorded
from .models import (
    AUTH_PAYLOAD,
    MAP_PAYLOAD,
    URL_PAYLOAD,
    AuthCase,
    ErrorProbe,
    MantleCalls,
    Object,
    UrlCase,
)

MODEL = "openai.gpt-oss-120b"


def url_value(calls: MantleCalls, case: UrlCase) -> JsonValue:
    params: Object = {}
    region = case.get("aws_region_name")
    if region is not None:
        params["aws_region_name"] = region
    with environment(case.get("environment")):
        try:
            return {"url": calls.get_complete_url(case.get("api_base"), params)}
        except ValueError:
            return {"error": True}


def url_cases(payload: JsonValue) -> JsonValue:
    urls: list[JsonValue] = []
    for case in URL_PAYLOAD.validate_python(payload)["cases"]:
        urls.append(url_value(config_of(case), case))
    return {"urls": urls}


def map_cases(payload: JsonValue, *, with_body: bool) -> JsonValue:
    results: list[JsonValue] = []
    for case in MAP_PAYLOAD.validate_python(payload)["cases"]:
        config = config_of(case)
        with warnings_recorded() as messages:
            result = map_result(config, case, with_body=with_body)
        results.append({**result, "warnings": messages})
    return {"results": results}


def error_value(probe: ErrorProbe | None) -> JsonValue:
    if probe is None:
        return None
    config = BedrockMantleResponsesAPIConfig()
    error = config.get_error_class(probe["message"], probe["status"], {})
    return {"status": error.status_code, "message": error.message}


def auth_value(calls: MantleCalls, case: AuthCase) -> JsonValue:
    api_key = case.get("api_key")
    project_id = case.get("project_id")
    given = case.get("headers")
    headers: Object = {} if given is None else {**given}
    litellm_params: GenericLiteLLMParams | None = None
    if api_key is not None or project_id is not None:
        litellm_params = GenericLiteLLMParams(api_key=api_key, aws_bedrock_project_id=project_id)
    calls.validate_environment(headers, MODEL, litellm_params)
    return {
        "headers": headers,
        "provider": calls.custom_llm_provider.value,
        "supports_native_file_search": calls.supports_native_file_search(),
        "supports_native_websocket": calls.supports_native_websocket(),
        "error": error_value(case.get("error")),
    }


def auth_cases(payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in AUTH_PAYLOAD.validate_python(payload)["cases"]:
        with environment(case.get("environment")):
            results.append(auth_value(BedrockMantleResponsesAPIConfig(), case))
    return {"results": results}
