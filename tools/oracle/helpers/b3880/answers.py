from typing import cast

from litellm.integrations import prometheus as prometheus_integration
from litellm.proxy import proxy_server
from litellm.router import Router
from litellm.types.integrations.prometheus import UserAPIKeyLabelValues
from pydantic import JsonValue

from .exceptions import build_exception
from .logger import LOGGER
from .router import CaseRouter
from .values import JSON_ADAPTER, OBJECT_ADAPTER, ExceptionCase, Input


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
    return LOGGER._extract_status_code(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L2426
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
    return LOGGER._should_skip_metrics_for_invalid_key(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L2518
        kwargs=kwargs_with_exception(input_value["kwargs_exception"]),
        enum_values=label_values(input_value["enum_status_code"]),
        exception=optional_exception(input_value["exception"]),
    )


def class_name_answer(input_value: Input) -> JsonValue:
    exception = required_exception(input_value["exception"])
    return LOGGER._get_exception_class_name(exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L3374


def rate_limit_answer(input_value: Input) -> JsonValue:
    exception = optional_exception(input_value["exception"])
    labels = LOGGER._extract_rate_limit_labels(exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L3413
    return JSON_ADAPTER.validate_python(list(labels))


def requested_model_answer(input_value: Input) -> JsonValue:
    catalog = input_value["catalog"]
    proxy_server.llm_router = None if catalog is None else cast(Router, CaseRouter(catalog))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2584
    return prometheus_integration._bounded_requested_model_label(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L217
        input_value["requested_model"],
        input_value["router_originated"],
    )


def request_provider_answer(input_value: Input) -> JsonValue:
    return LOGGER._extract_api_provider_from_request_data(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L2562
        request_data(input_value)
    )


def exception_provider_answer(input_value: Input) -> JsonValue:
    exception = required_exception(input_value["exception"])
    return LOGGER._extract_api_provider_from_exception(exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L2599
