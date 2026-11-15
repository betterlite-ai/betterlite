import requests.sessions
from litellm.llms.databricks.chat.transformation import DatabricksConfig
from litellm.llms.databricks.common_utils import DatabricksBase
from pydantic import JsonValue, TypeAdapter

from .common import (
    JSON_ADAPTER,
    ApiBaseCase,
    AuthApi,
    ChatApi,
    Input,
    RecordingAdapter,
    ResponsesApi,
    UserAgentCase,
    ValidateCase,
    cases_of,
    guarded,
)
from .responses import ResponsesConfig

GATEWAY_MODEL = "a.b.c"


def environment(base: AuthApi, case: ValidateCase) -> dict[str, JsonValue]:
    endpoint = case.get("endpoint")
    if endpoint is None:
        endpoint = "chat_completions"
    url, headers = base.databricks_validate_environment(
        case.get("api_key"),
        case.get("api_base"),
        endpoint,
        case.get("custom_endpoint"),
        case.get("headers"),
        case.get("custom_user_agent"),
    )
    return {"url": url, "headers": JSON_ADAPTER.validate_python(headers)}


def validate(input_value: Input) -> JsonValue:
    cases = TypeAdapter(list[ValidateCase]).validate_python(cases_of(input_value))
    base: AuthApi = DatabricksBase()
    results: list[JsonValue] = []
    requests.sessions.HTTPAdapter = RecordingAdapter
    for case in cases:
        oauth = case.get("oauth")
        RecordingAdapter.script = {} if oauth is None else oauth
        RecordingAdapter.sent = None
        outcome = guarded(environment, base, case)
        outcome["request"] = RecordingAdapter.sent
        results.append(outcome)
    return results


def gateway_url(chat: ChatApi, responses: ResponsesApi, case: ApiBaseCase) -> dict[str, JsonValue]:
    if case["use_ai_gateway"]:
        url = chat.get_complete_url(case["api_base"], None, GATEWAY_MODEL, {}, {})
    else:
        url = responses.get_complete_url(case["api_base"], {})
    return {"url": url}


def api_base(input_value: Input) -> JsonValue:
    cases = TypeAdapter(list[ApiBaseCase]).validate_python(cases_of(input_value))
    chat = DatabricksConfig()
    responses = ResponsesConfig()
    return [guarded(gateway_url, chat, responses, case) for case in cases]


def agent_of(base: AuthApi, case: UserAgentCase) -> str:
    custom_endpoint = False
    _, headers = base.databricks_validate_environment(
        "key", "https://host", "chat_completions", custom_endpoint, None, case.get("custom")
    )
    return headers["User-Agent"]


def user_agent(input_value: Input) -> JsonValue:
    cases = TypeAdapter(list[UserAgentCase]).validate_python(cases_of(input_value))
    base = DatabricksBase()
    return [agent_of(base, case) for case in cases]


def redaction(input_value: Input) -> JsonValue:
    headers = input_value.get("headers")
    if headers is None:
        raise ValueError("headers is required")
    base = DatabricksBase()
    redacted_headers = base.redact_headers_for_logging(headers)
    redacted_data = base.redact_sensitive_data(input_value.get("data"))
    result: dict[str, JsonValue] = {
        "headers": JSON_ADAPTER.validate_python(redacted_headers),
        "data": JSON_ADAPTER.validate_python(redacted_data),
    }
    return result
