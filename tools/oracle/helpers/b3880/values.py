from typing import Final, Literal, TypedDict

from pydantic import JsonValue, TypeAdapter

JSON_ADAPTER: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECT_ADAPTER: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])

Function = Literal[
    "extract_status_code",
    "is_invalid_api_key_request",
    "should_skip_metrics_for_invalid_key",
    "extract_deployment_failure_label_values",
    "get_exception_class_name",
    "bounded_requested_model_label",
    "extract_rate_limit_labels",
    "extract_api_provider_from_request_data",
    "extract_api_provider_from_exception",
]


class ExceptionCase(TypedDict):
    kind: Literal["scripted", "rate_limit", "budget_exceeded", "proxy_rate_limit"]
    message: str | None
    status_code: str | None
    code: str | None
    llm_provider: str | None
    category: str | None
    rate_limit_type: str | None
    legacy_class_name: str | None


class AuthCase(TypedDict):
    api_key: str | None
    user_email: str | None
    key_alias: str | None
    team_id: str | None
    team_alias: str | None


class RouterCase(TypedDict):
    recognized: list[str]
    team_public: list[str]
    patterns: list[str]
    team_patterns: list[str]


class Input(TypedDict):
    function: Function
    enum_status_code: str | None
    status_code: int | None
    exception: ExceptionCase | None
    kwargs_exception: ExceptionCase | None
    payload_metadata: dict[str, str | None] | None
    litellm_params_metadata: dict[str, str | None] | None
    user_api_key_auth: AuthCase | None
    requested_model: JsonValue
    router_originated: bool
    catalog: RouterCase | None
    litellm_params: dict[str, str | None] | None
    standard_logging: dict[str, str | None] | None
    model: str | None
