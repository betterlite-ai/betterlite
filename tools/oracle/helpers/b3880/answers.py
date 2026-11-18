from typing import Final, Literal, Protocol, TypedDict, cast

from litellm.integrations import prometheus as prometheus_integration
from litellm.proxy import proxy_server
from litellm.router import Router
from litellm.types.integrations.prometheus import UserAPIKeyLabelValues
from pydantic import JsonValue, TypeAdapter

from .exceptions import ExceptionCase, build_exception
from .logger import LOGGER

JSON_ADAPTER: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECT_ADAPTER: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
RATE_LIMIT_LABELS: Final[TypeAdapter[tuple[str | None, str | None]]] = TypeAdapter(
    tuple[str | None, str | None]
)

Function = Literal[
    "extract_status_code",
    "is_invalid_api_key_request",
    "should_skip_metrics_for_invalid_key",
    "deployment_failure_labels",
    "get_exception_class_name",
    "bounded_requested_model_label",
    "extract_rate_limit_labels",
    "api_provider_from_request_data",
    "api_provider_from_exception",
]


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


class RouterView(Protocol):
    def is_recognized_model(self, model: str) -> bool: ...


class CasePatternRouter:
    def __init__(self, patterns: list[str]) -> None:
        self.patterns = patterns

    def route(self, model: str) -> str | None:
        for pattern in self.patterns:
            if pattern.endswith("*"):
                matched = model.startswith(pattern.removesuffix("*"))
            else:
                matched = model == pattern
            if matched:
                return model
        return None


class CaseRouter(RouterView):
    def __init__(self, case: RouterCase) -> None:
        self.recognized: set[str] = set(case["recognized"])
        self.team_public_model_names: list[str] = list(case["team_public"])
        self.pattern_router = CasePatternRouter(case["patterns"])
        self.team_pattern_routers: dict[str, CasePatternRouter] = {
            "team": CasePatternRouter(case["team_patterns"])
        }

    def is_recognized_model(self, model: str) -> bool:
        return model in self.recognized


def optional_exception(case: ExceptionCase | None) -> Exception | None:
    if case is None:
        return None
    return build_exception(case)


def required_exception(case: ExceptionCase | None) -> Exception:
    if case is None:
        raise ValueError("the case names an exception")
    return build_exception(case)


def label_values(status_code: str | None) -> UserAPIKeyLabelValues | None:
    if status_code is None:
        return None
    return UserAPIKeyLabelValues(status_code=status_code)


def kwargs_with_exception(case: ExceptionCase | None) -> dict[str, Exception] | None:
    exception = optional_exception(case)
    if exception is None:
        return None
    return {"exception": exception}


def request_data(input_value: Input) -> dict[str, JsonValue]:
    data: dict[str, JsonValue] = {}
    litellm_params = input_value["litellm_params"]
    standard_logging = input_value["standard_logging"]
    model = input_value["model"]
    if litellm_params is not None:
        data["litellm_params"] = OBJECT_ADAPTER.validate_python(litellm_params)
    if standard_logging is not None:
        data["standard_logging_object"] = OBJECT_ADAPTER.validate_python(standard_logging)
    if model is not None:
        data["model"] = model
    return data


def status_code_answer(input_value: Input) -> JsonValue:
    return LOGGER._extract_status_code(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L2426
        kwargs=kwargs_with_exception(input_value["kwargs_exception"]),
        enum_values=label_values(input_value["enum_status_code"]),
        exception=optional_exception(input_value["exception"]),
    )


def invalid_key_answer(input_value: Input) -> JsonValue:
    return LOGGER._is_invalid_api_key_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L2479
        status_code=input_value["status_code"],
        exception=optional_exception(input_value["exception"]),
    )


def skip_answer(input_value: Input) -> JsonValue:
    return LOGGER._should_skip_metrics_for_invalid_key(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L2518
        kwargs=kwargs_with_exception(input_value["kwargs_exception"]),
        enum_values=label_values(input_value["enum_status_code"]),
        exception=optional_exception(input_value["exception"]),
    )


def class_name_answer(input_value: Input) -> JsonValue:
    exception = required_exception(input_value["exception"])
    return LOGGER._get_exception_class_name(exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L3374


def rate_limit_answer(input_value: Input) -> JsonValue:
    exception = optional_exception(input_value["exception"])
    labels = RATE_LIMIT_LABELS.validate_python(
        LOGGER._extract_rate_limit_labels(exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L3413
    )
    return JSON_ADAPTER.validate_python(list(labels))


def requested_model_answer(input_value: Input) -> JsonValue:
    catalog = input_value["catalog"]
    proxy_server.llm_router = None if catalog is None else cast(Router, CaseRouter(catalog))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2584
    return prometheus_integration._bounded_requested_model_label(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L217
        input_value["requested_model"],
        input_value["router_originated"],
    )


def request_provider_answer(input_value: Input) -> JsonValue:
    return LOGGER._extract_api_provider_from_request_data(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L2562
        request_data(input_value)
    )


def exception_provider_answer(input_value: Input) -> JsonValue:
    exception = required_exception(input_value["exception"])
    return LOGGER._extract_api_provider_from_exception(exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L2599
