import logging
import os
from collections.abc import Callable
from typing import Final, NotRequired, TypedDict, TypeAlias

import litellm
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.bedrock_mantle.responses.transformation import BedrockMantleResponsesAPIConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

os.environ["LITELLM_RUST"] = "false"

ENVIRONMENT_KEYS: Final = (
    "AWS_BEARER_TOKEN_BEDROCK",
    "AWS_REGION",
    "AWS_REGION_NAME",
    "BEDROCK_MANTLE_API_BASE",
    "BEDROCK_MANTLE_API_KEY",
    "BEDROCK_MANTLE_REGION",
)

UrlCall: TypeAlias = Callable[[str | None, dict[str, JsonValue]], str]
MapCall: TypeAlias = Callable[[dict[str, JsonValue], str, bool], dict[str, JsonValue]]
EnvironmentCall: TypeAlias = Callable[
    [dict[str, JsonValue], str, GenericLiteLLMParams | None], dict[str, JsonValue]
]
TransformCall: TypeAlias = Callable[
    [str, JsonValue, dict[str, JsonValue], GenericLiteLLMParams, dict[str, str]],
    dict[str, JsonValue],
]
ErrorClassCall: TypeAlias = Callable[[str, int, dict[str, str]], BaseLLMException]

URL: Final = TypeAdapter(UrlCall)
MAP: Final = TypeAdapter(MapCall)
ENVIRONMENT: Final = TypeAdapter(EnvironmentCall)
TRANSFORM: Final = TypeAdapter(TransformCall)
ERROR_CLASS: Final = TypeAdapter(ErrorClassCall)


class UrlCase(TypedDict):
    api_base: NotRequired[str]
    aws_region_name: NotRequired[str]
    use_openai_path: NotRequired[bool]
    environment: NotRequired[dict[str, str]]


class MapCase(TypedDict):
    model: str
    params: NotRequired[dict[str, JsonValue]]
    drop_params: NotRequired[bool]
    use_openai_path: NotRequired[bool]


class ErrorProbe(TypedDict):
    status: int
    message: str
    headers: NotRequired[dict[str, str]]


class AuthCase(TypedDict):
    api_key: NotRequired[str]
    project_id: NotRequired[str]
    headers: NotRequired[dict[str, str]]
    error: NotRequired[ErrorProbe]
    environment: NotRequired[dict[str, str]]


class HoistCase(TypedDict):
    model: str
    input: JsonValue
    params: NotRequired[dict[str, JsonValue]]


class UrlPayload(TypedDict):
    cases: list[UrlCase]


class MapPayload(TypedDict):
    cases: list[MapCase]


class AuthPayload(TypedDict):
    cases: list[AuthCase]


class HoistPayload(TypedDict):
    cases: list[HoistCase]


URL_PAYLOAD: Final = TypeAdapter(UrlPayload)
MAP_PAYLOAD: Final = TypeAdapter(MapPayload)
AUTH_PAYLOAD: Final = TypeAdapter(AuthPayload)
HOIST_PAYLOAD: Final = TypeAdapter(HoistPayload)


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def enter_environment(values: dict[str, str] | None) -> dict[str, str | None]:
    saved: dict[str, str | None] = {}
    for name in ENVIRONMENT_KEYS:
        saved[name] = os.environ.get(name)
        os.environ.pop(name, None)
    if values is None:
        return saved
    for name in ENVIRONMENT_KEYS:
        value = values.get(name)
        if value is not None:
            os.environ[name] = value
    return saved


def leave_environment(saved: dict[str, str | None]) -> None:
    for name in ENVIRONMENT_KEYS:
        os.environ.pop(name, None)
        value = saved[name]
        if value is not None:
            os.environ[name] = value


def config_of(case: UrlCase | MapCase) -> BedrockMantleResponsesAPIConfig:
    use_openai_path = case.get("use_openai_path")
    if use_openai_path is None:
        use_openai_path = True
    return BedrockMantleResponsesAPIConfig(use_openai_path=use_openai_path)


def params_of(case: MapCase) -> dict[str, JsonValue]:
    params = case.get("params")
    if params is None:
        return {}
    return params


def drop_of(case: MapCase) -> bool:
    drop_params = case.get("drop_params")
    if drop_params is None:
        return False
    return drop_params


def url_cases(payload: JsonValue) -> JsonValue:
    cases = URL_PAYLOAD.validate_python(payload)["cases"]
    urls: list[JsonValue] = []
    for case in cases:
        saved = enter_environment(case.get("environment"))
        try:
            params: dict[str, JsonValue] = {}
            region = case.get("aws_region_name")
            if region is not None:
                params["aws_region_name"] = region
            config = config_of(case)
            url = URL.validate_python(config.get_complete_url)(case.get("api_base"), params)
        except ValueError:
            urls.append({"error": True})
        else:
            urls.append({"url": url})
        finally:
            leave_environment(saved)
    return {"urls": urls}


def map_case(case: MapCase) -> JsonValue:
    config = config_of(case)
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    try:
        mapped = MAP.validate_python(config.map_openai_params)(
            params_of(case), case["model"], drop_of(case)
        )
    except litellm.UnsupportedParamsError as error:
        return {"error": {"status": error.status_code}, "warnings": warnings.messages}
    finally:
        logger.removeHandler(warnings)
    return {"params": mapped, "warnings": warnings.messages}


def map_cases(payload: JsonValue) -> JsonValue:
    cases = MAP_PAYLOAD.validate_python(payload)["cases"]
    results: list[JsonValue] = []
    for case in cases:
        results.append(map_case(case))
    return {"results": results}


def model_name_case(case: MapCase) -> JsonValue:
    config = config_of(case)
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    try:
        mapped = MAP.validate_python(config.map_openai_params)(
            params_of(case), case["model"], drop_of(case)
        )
        body = TRANSFORM.validate_python(config.transform_responses_api_request)(
            case["model"], "hello", mapped, GenericLiteLLMParams(), {}
        )
    except litellm.UnsupportedParamsError as error:
        return {"error": {"status": error.status_code}, "warnings": warnings.messages}
    finally:
        logger.removeHandler(warnings)
    return {"body": body, "warnings": warnings.messages}


def model_name_cases(payload: JsonValue) -> JsonValue:
    cases = MAP_PAYLOAD.validate_python(payload)["cases"]
    results: list[JsonValue] = []
    for case in cases:
        results.append(model_name_case(case))
    return {"results": results}


def litellm_params_of(case: AuthCase) -> GenericLiteLLMParams | None:
    api_key = case.get("api_key")
    project_id = case.get("project_id")
    if api_key is None and project_id is None:
        return None
    return GenericLiteLLMParams(api_key=api_key, aws_bedrock_project_id=project_id)


def input_headers(case: AuthCase | ErrorProbe) -> dict[str, JsonValue]:
    headers: dict[str, JsonValue] = {}
    given = case.get("headers")
    if given is None:
        return headers
    for name, value in given.items():
        headers[name] = value
    return headers


def error_value(probe: ErrorProbe | None) -> JsonValue:
    if probe is None:
        return None
    config = BedrockMantleResponsesAPIConfig()
    error = ERROR_CLASS.validate_python(config.get_error_class)(probe["message"], probe["status"], {})
    return {"status": error.status_code, "message": error.message}


def auth_case(case: AuthCase) -> JsonValue:
    config = BedrockMantleResponsesAPIConfig()
    headers = input_headers(case)
    ENVIRONMENT.validate_python(config.validate_environment)(
        headers, "openai.gpt-oss-120b", litellm_params_of(case)
    )
    return {
        "headers": headers,
        "provider": config.custom_llm_provider.value,
        "supports_native_file_search": config.supports_native_file_search(),
        "supports_native_websocket": config.supports_native_websocket(),
        "error": error_value(case.get("error")),
    }


def auth_cases(payload: JsonValue) -> JsonValue:
    cases = AUTH_PAYLOAD.validate_python(payload)["cases"]
    results: list[JsonValue] = []
    for case in cases:
        saved = enter_environment(case.get("environment"))
        try:
            results.append(auth_case(case))
        finally:
            leave_environment(saved)
    return {"results": results}


def hoist_case(case: HoistCase) -> JsonValue:
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    try:
        params = case.get("params")
        if params is None:
            params = {}
        transform = TRANSFORM.validate_python(
            BedrockMantleResponsesAPIConfig().transform_responses_api_request
        )
        body = transform(case["model"], case["input"], params, GenericLiteLLMParams(), {})
    finally:
        logger.removeHandler(warnings)
    return {"body": body, "warnings": warnings.messages}


def hoist_cases(payload: JsonValue) -> JsonValue:
    cases = HOIST_PAYLOAD.validate_python(payload)["cases"]
    results: list[JsonValue] = []
    for case in cases:
        results.append(hoist_case(case))
    return {"results": results}


HANDLERS: Final[dict[str, Callable[[JsonValue], JsonValue]]] = {
    "mantle_responses_url": url_cases,
    "mantle_response_options": map_cases,
    "mantle_model_name": model_name_cases,
    "unsupported_params_400": map_cases,
    "mantle_response_auth": auth_cases,
    "unsupported_params_warn_drop": map_cases,
    "openai_project_header": auth_cases,
    "mantle_tool_filter": map_cases,
    "mantle_input_hoisting": hoist_cases,
}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    if len(input_value) != 1:
        raise ValueError("bedrock_mantle_responses takes exactly one operation")
    operation, payload = next(iter(input_value.items()))
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    return handler(payload)
