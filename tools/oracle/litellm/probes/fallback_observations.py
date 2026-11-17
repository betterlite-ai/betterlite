import asyncio
import warnings
from collections.abc import Callable, Coroutine
from typing import Annotated, Final, Literal

import litellm
from litellm.exceptions import (
    BadRequestError,
    BudgetExceededError,
    GuardrailRaisedException,
    JSONSchemaValidationError,
    RateLimitError,
)
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.secret_redaction import redact_string
from litellm.router_utils import fallback_event_handlers
from litellm.router_utils.add_retry_fallback_headers import (
    FallbackErrorInfo,
    add_fallback_headers_to_response,
    ensure_response_additional_headers,
    get_fallback_error_info,
    get_fallback_errors_from_headers,
)
from litellm.router_utils.common_utils import (
    format_fallback_outcome_message,
    format_no_fallback_group_message,
    truncate_fallback_error_detail,
)
from pydantic import Field, InstanceOf, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

MODEL: Final = "gpt-4"
PROVIDER: Final = "openai"
ERRORS_HEADER: Final = "x-litellm-fallback-errors"
HEADERS: Final = TypeAdapter(dict[str, JsonValue])
ERRORS: Final = TypeAdapter(list[dict[str, JsonValue]])
# litellm.callbacks mixes callables, names and loggers, so the probe claims only that it is a list
CALLBACKS: Final[TypeAdapter[list[object]]] = TypeAdapter(InstanceOf[list[object]])
LOG_SUCCESS: Callable[..., Coroutine[object, object, None]] = TypeAdapter(
    Callable[..., Coroutine[object, object, None]]
).validate_python(fallback_event_handlers.__dict__["log_success_fallback_event"])
LOG_FAILURE: Callable[..., Coroutine[object, object, None]] = TypeAdapter(
    Callable[..., Coroutine[object, object, None]]
).validate_python(fallback_event_handlers.__dict__["log_failure_fallback_event"])
ObserverKind = Literal["record", "fail", "silent"]
OutcomeKind = Literal["success", "failure"]
ResponseKind = Literal["absent", "object", "attributed", "opaque"]


class RateLimitCase(Strict):
    family: Literal["rate_limit"]
    message: str
    body_param: str | None = None


class BadRequestCase(Strict):
    family: Literal["bad_request"]
    message: str
    body_type: str | None = None
    body_param: str | None = None


class BudgetCase(Strict):
    family: Literal["budget"]
    message: str | None = None
    current_cost: str
    max_budget: str


class GuardrailCase(Strict):
    family: Literal["guardrail"]
    message: str
    guardrail_name: str | None = None


class JsonSchemaCase(Strict):
    family: Literal["json_schema"]
    raw_response: str
    json_schema: str


class PlainCase(Strict):
    family: Literal["plain"]
    message: str
    exception_class: Literal["exception", "value_error"] = "exception"


DetailCase = Annotated[
    RateLimitCase | BadRequestCase | BudgetCase | GuardrailCase | JsonSchemaCase | PlainCase,
    Field(discriminator="family"),
]


class NoFallbackCase(Strict):
    lookup_groups: list[str]
    fallbacks: list[dict[str, JsonValue]]


class OutcomeCase(Strict):
    model_group: str | None
    fallback_model_group: list[JsonValue] | None
    fallback_failure_detail: str


class InfoInput(Strict):
    message: str
    type: str
    param: str | None
    code: str | None


class HeaderCase(Strict):
    kind: ResponseKind
    attempted_fallbacks: int
    errors: list[InfoInput] | None = None
    existing: dict[str, JsonValue] | None = None


class EventCase(Strict):
    observers: list[ObserverKind]
    outcome: OutcomeKind
    original_model_group: str
    request: dict[str, JsonValue]
    message: str


class InfoFound(Strict):
    message: str
    type: str
    param: str | None
    code: str | None


class DetailFound(Strict):
    info: InfoFound
    detail: str


class HeaderFound(Strict):
    result: ResponseKind
    has_hidden_params: bool
    headers: dict[str, JsonValue]
    errors_present: bool
    errors: list[dict[str, JsonValue]]


class EventCall(Strict):
    observer: int
    outcome: OutcomeKind
    original_model_group: str
    model: str | None
    message: str


class EventFound(Strict):
    calls: list[EventCall]


class Output(Strict):
    details: list[DetailFound]
    no_fallback: list[str]
    outcomes: list[str]
    headers: list[HeaderFound]
    events: list[EventFound]


class Attributed:
    def __init__(self, hidden: dict[str, JsonValue]) -> None:
        self._hidden_params: dict[str, JsonValue] = hidden

    def hidden_params(self) -> dict[str, JsonValue]:
        return self._hidden_params


class Opaque:
    pass


class RecordingLogger(CustomLogger):
    def __init__(self, index: int, calls: list[EventCall]) -> None:
        # CustomLogger.__init__ only records message-logging flags this probe never reads
        self.index: int = index
        self.calls: list[EventCall] = calls

    def record(
        self, outcome: OutcomeKind, original_model_group: str, kwargs: dict[str, object], original_exception: Exception
    ) -> None:
        model = kwargs.get("model")
        self.calls.append(
            EventCall(
                observer=self.index,
                outcome=outcome,
                original_model_group=original_model_group,
                model=model if isinstance(model, str) else None,
                message=str(original_exception),
            )
        )

    @override
    async def log_success_fallback_event(
        self, original_model_group: str, kwargs: dict[str, object], original_exception: Exception
    ) -> None:
        self.record("success", original_model_group, kwargs, original_exception)

    @override
    async def log_failure_fallback_event(
        self, original_model_group: str, kwargs: dict[str, object], original_exception: Exception
    ) -> None:
        self.record("failure", original_model_group, kwargs, original_exception)


def failure_error(
    original_model_group: str, kwargs: dict[str, object], original_exception: Exception
) -> RuntimeError:
    return RuntimeError(
        f"the observer failed for {original_model_group} over {len(kwargs)} values: {original_exception}"
    )


class FailingLogger(CustomLogger):
    @override
    async def log_success_fallback_event(
        self,
        original_model_group: str,
        kwargs: dict[str, object],
        original_exception: Exception,
    ) -> None:
        raise failure_error(original_model_group, kwargs, original_exception)

    @override
    async def log_failure_fallback_event(
        self,
        original_model_group: str,
        kwargs: dict[str, object],
        original_exception: Exception,
    ) -> None:
        raise failure_error(original_model_group, kwargs, original_exception)


class SilentLogger(CustomLogger):
    pass


def body_fields(body_type: str | None, body_param: str | None) -> dict[str, object] | None:
    if body_type is None and body_param is None:
        return None
    fields: dict[str, object] = {}
    if body_type is not None:
        fields["type"] = body_type
    if body_param is not None:
        fields["param"] = body_param
    return fields


def exception_for(case: DetailCase) -> Exception:
    if isinstance(case, RateLimitCase):
        return RateLimitError(
            message=case.message, llm_provider=PROVIDER, model=MODEL, body=body_fields(None, case.body_param)
        )
    if isinstance(case, BadRequestCase):
        return BadRequestError(
            message=case.message,
            llm_provider=PROVIDER,
            model=MODEL,
            body=body_fields(case.body_type, case.body_param),
        )
    if isinstance(case, BudgetCase):
        return BudgetExceededError(
            current_cost=float(case.current_cost), max_budget=float(case.max_budget), message=case.message
        )
    if isinstance(case, GuardrailCase):
        return GuardrailRaisedException(guardrail_name=case.guardrail_name, message=case.message)
    if isinstance(case, JsonSchemaCase):
        return JSONSchemaValidationError(
            model=MODEL, llm_provider=PROVIDER, raw_response=case.raw_response, schema=case.json_schema
        )
    return plain_exception(case)


def plain_exception(case: PlainCase) -> Exception:
    if case.exception_class == "value_error":
        return ValueError(case.message)
    return Exception(case.message)


def detail_for(case: DetailCase) -> DetailFound:
    error = exception_for(case)
    info = get_fallback_error_info(error)
    return DetailFound(
        info=InfoFound(message=info["message"], type=info["type"], param=info["param"], code=info["code"]),
        detail=truncate_fallback_error_detail(redact_string(str(error))),
    )


def no_fallback_message(case: NoFallbackCase) -> str:
    return format_no_fallback_group_message(case.lookup_groups, case.fallbacks)


def outcome_message(case: OutcomeCase) -> str:
    return format_fallback_outcome_message(
        case.model_group, case.fallback_model_group, case.fallback_failure_detail
    )


def error_info(info: InfoInput) -> FallbackErrorInfo:
    return FallbackErrorInfo(message=info.message, type=info.type, param=info.param, code=info.code)


def header_found(result: ResponseKind, response: object, *, has_hidden_params: bool) -> HeaderFound:
    additional = ensure_response_additional_headers(response)
    visible = HEADERS.validate_python({key: value for key, value in additional.items() if key != ERRORS_HEADER})
    return HeaderFound(
        result=result,
        has_hidden_params=has_hidden_params,
        headers=visible,
        errors_present=ERRORS_HEADER in additional,
        errors=ERRORS.validate_python(get_fallback_errors_from_headers(additional)),
    )


def header_for(case: HeaderCase) -> HeaderFound:
    errors = None if case.errors is None else [error_info(item) for item in case.errors]
    if case.kind == "absent":
        _ = add_fallback_headers_to_response(None, case.attempted_fallbacks, errors)
        return header_found("absent", None, has_hidden_params=False)
    if case.kind == "object":
        payload: dict[str, JsonValue] = {}
        if case.existing is not None:
            payload["_hidden_params"] = {"additional_headers": dict(case.existing)}
        _ = add_fallback_headers_to_response(payload, case.attempted_fallbacks, errors)
        return header_found("object", payload, has_hidden_params="_hidden_params" in payload)
    if case.kind == "attributed":
        hidden: dict[str, JsonValue] = {} if case.existing is None else {"additional_headers": dict(case.existing)}
        attributed = Attributed(hidden)
        _ = add_fallback_headers_to_response(attributed, case.attempted_fallbacks, errors)
        return header_found("attributed", attributed, has_hidden_params=True)
    opaque = Opaque()
    _ = add_fallback_headers_to_response(opaque, case.attempted_fallbacks, errors)
    return header_found("opaque", opaque, has_hidden_params=False)


def logger_for(kind: ObserverKind, index: int, calls: list[EventCall]) -> CustomLogger:
    if kind == "record":
        return RecordingLogger(index, calls)
    if kind == "fail":
        return FailingLogger()
    return SilentLogger()


async def event_for(case: EventCase) -> EventFound:
    calls: list[EventCall] = []
    loggers = [logger_for(kind, index, calls) for index, kind in enumerate(case.observers)]
    exception = Exception(case.message)
    callbacks = CALLBACKS.validate_python(litellm.__dict__["callbacks"])
    before = len(callbacks)
    callbacks.extend(loggers)
    try:
        if case.outcome == "success":
            await LOG_SUCCESS(
                original_model_group=case.original_model_group,
                kwargs=dict(case.request),
                original_exception=exception,
            )
        else:
            await LOG_FAILURE(
                original_model_group=case.original_model_group,
                kwargs=dict(case.request),
                original_exception=exception,
            )
    finally:
        del callbacks[before:]
    return EventFound(calls=calls)


class FallbackObservations(Operation):
    details: list[DetailCase] = Field(default_factory=list)
    no_fallback: list[NoFallbackCase] = Field(default_factory=list)
    outcomes: list[OutcomeCase] = Field(default_factory=list)
    headers: list[HeaderCase] = Field(default_factory=list)
    events: list[EventCase] = Field(default_factory=list)

    @override
    def run(self) -> Output:
        # importing LiteLLM emits no warning, but its logging models emit ReadOnly qualifiers while they build;
        # neither side compares warnings, so the probe drops them
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return asyncio.run(self.collect())

    async def collect(self) -> Output:
        return Output(
            details=[detail_for(case) for case in self.details],
            no_fallback=[no_fallback_message(case) for case in self.no_fallback],
            outcomes=[outcome_message(case) for case in self.outcomes],
            headers=[header_for(case) for case in self.headers],
            events=[await event_for(case) for case in self.events],
        )
