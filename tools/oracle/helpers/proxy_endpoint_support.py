import logging
from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from fastapi import HTTPException
from litellm.litellm_core_utils.bug_report import NOTICE_PREFIX
from litellm.proxy._types import ProxyException
from litellm.proxy.common_utils.openai_error_payload import litellm_call_id_headers
from litellm.proxy.utils import handle_exception_on_proxy, is_known_model
from litellm.router import Router
from pydantic import JsonValue
from typing_extensions import override

DEFAULT_MODEL: Final = "openai/gpt-4o"
PROXY_LOGGER_NAME: Final = "LiteLLM Proxy"


class HttpSpec(TypedDict):
    kind: Literal["http"]
    status_code: int
    detail: str


class ProxySpec(TypedDict):
    kind: Literal["proxy"]
    message: str
    error_type: str
    param: str | None
    code: int | None
    headers: dict[str, str]


class FailureSpec(TypedDict):
    kind: Literal["failure"]
    message: str
    status_code: int | None
    param: str | None


ExceptionSpec: TypeAlias = HttpSpec | ProxySpec | FailureSpec


class RecordCall(TypedDict):
    function: Literal["record"]
    exception: ExceptionSpec
    litellm_call_id: str | None
    notice: bool


class KnownModelCall(TypedDict):
    function: Literal["known_model"]
    models: list[str] | None
    lookups: list[str | None]


Call: TypeAlias = RecordCall | KnownModelCall


class Input(TypedDict):
    calls: list[Call]


class RecordView(TypedDict):
    message: str
    type: str
    param: str | None
    code: str
    headers: dict[str, str]


class RecordAnswer(TypedDict):
    record: RecordView
    exception_logged: bool
    notice_logged: bool


class NoticeMismatchError(RuntimeError):
    pass


# status_code is set only when the case supplies one, so getattr answers None otherwise
class FailureError(Exception):
    def __init__(self, message: str, status_code: int | None, param: str | None) -> None:
        super().__init__(message)
        if status_code is not None:
            self.status_code = status_code
        if param is not None:
            self.param = param


class Recorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.messages: list[tuple[int, str]] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append((record.levelno, record.getMessage()))


def proxy_value(spec: ProxySpec) -> tuple[ProxyException, dict[str, str]]:
    headers = dict(spec["headers"])
    raised = ProxyException(
        message=spec["message"],
        type=spec["error_type"],
        param=spec["param"],
        code=spec["code"],
        headers=headers,
    )
    return raised, headers


def caught_exception(spec: ExceptionSpec) -> tuple[Exception, dict[str, str]]:
    match spec["kind"]:
        case "http":
            return HTTPException(status_code=spec["status_code"], detail=spec["detail"]), {}
        case "proxy":
            return proxy_value(spec)
        case "failure":
            return FailureError(spec["message"], spec["status_code"], spec["param"]), {}


def observed_headers(owned: dict[str, str], litellm_call_id: str | None) -> dict[str, str]:
    if owned:
        return owned
    headers = litellm_call_id_headers(litellm_call_id)
    return {} if headers is None else headers


def record_view(record: ProxyException, headers: dict[str, str]) -> RecordView:
    return {
        "message": record.message,
        "type": record.type,
        "param": record.param,
        "code": record.code,
        "headers": headers,
    }


def observed_record(
    raised: Exception, litellm_call_id: str | None, owned: dict[str, str]
) -> RecordAnswer:
    recorder = Recorder()
    proxy_logger = logging.getLogger(PROXY_LOGGER_NAME)
    installed = list(proxy_logger.handlers)
    proxy_logger.handlers = [recorder]
    try:
        handled = handle_exception_on_proxy(raised, litellm_call_id)
    finally:
        proxy_logger.handlers = installed
    messages = [message for level, message in recorder.messages if level == logging.ERROR]
    headers = observed_headers(owned, litellm_call_id)
    return {
        "record": record_view(handled, headers),
        "exception_logged": "Exception: " + str(raised) in messages,
        "notice_logged": any(message.startswith(NOTICE_PREFIX) for message in messages),
    }


def record_answer(call: RecordCall) -> RecordAnswer:
    raised, owned = caught_exception(call["exception"])
    outcome = observed_record(raised, call["litellm_call_id"], owned)
    if outcome["notice_logged"] != call["notice"]:
        raise NoticeMismatchError("LiteLLM logged a notice the case did not script")
    return outcome


def known_model_answer(call: KnownModelCall) -> list[bool]:
    models = call["models"]
    if models is None:
        return [is_known_model(model, None) for model in call["lookups"]]
    model_list: list[dict[str, JsonValue]] = [
        {"model_name": name, "litellm_params": {"model": DEFAULT_MODEL}} for name in models
    ]
    router = Router(model_list=model_list)
    return [is_known_model(model, router) for model in call["lookups"]]


def answer(call: Call) -> RecordAnswer | list[bool]:
    match call["function"]:
        case "record":
            return record_answer(call)
        case "known_model":
            return known_model_answer(call)


def run(input_value: Input) -> dict[str, list[RecordAnswer | list[bool]]]:
    litellm.suppress_debug_info = True
    return {"calls": [answer(call) for call in input_value["calls"]]}
