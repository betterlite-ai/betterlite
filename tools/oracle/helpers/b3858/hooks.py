from datetime import UTC, datetime
from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.integrations.opentelemetry import OpenTelemetry
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import ManagementEndpointLoggingPayload, UserAPIKeyAuth
from litellm.types.services import ServiceLoggerPayload, ServiceTypes
from litellm.types.utils import ModelResponse
from opentelemetry.trace import Span
from pydantic import JsonValue, TypeAdapter

DICT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
CarriedMetadata: TypeAlias = dict[str, JsonValue | Span]
CarriedParams: TypeAlias = dict[str, JsonValue | CarriedMetadata]
CarriedKwargs: TypeAlias = dict[str, JsonValue | CarriedMetadata | CarriedParams]
CarriedRequest: TypeAlias = dict[str, JsonValue | CarriedMetadata]
Mode: TypeAlias = Literal["success", "failure"]


class Parent(TypedDict):
    name: str
    start: NotRequired[float]
    end: NotRequired[float]


class ExceptionSpec(TypedDict):
    kind: Literal["authentication", "value_error"]
    message: str


class Team(TypedDict):
    id: NotRequired[str]
    alias: NotRequired[str]


class Common(TypedDict):
    start_time: float
    end_time: float
    parent: NotRequired[Parent]


class ServiceFields(Common):
    function: Literal["service_success", "service_failure"]
    service: str
    call_type: str
    caller: NotRequired[str | None]
    error: NotRequired[str | None]
    event_metadata: NotRequired[dict[str, JsonValue] | None]


class PostCallFailure(Common):
    function: Literal["post_call_failure"]
    kwargs: dict[str, JsonValue]
    request_data: dict[str, JsonValue]
    exception: ExceptionSpec
    traceback: NotRequired[str | None]
    team: NotRequired[Team]
    after_failure_log: NotRequired[bool]


class PostCallSuccess(Common):
    function: Literal["post_call_success"]
    kwargs: dict[str, JsonValue]
    key_parent: NotRequired[Parent]


class ManagementFields(Common):
    function: Literal["management_success", "management_failure"]
    route: str
    request_data: dict[str, JsonValue]
    response: NotRequired[dict[str, JsonValue]]
    exception: NotRequired[ExceptionSpec]


class ErrorAttributes(Common):
    function: Literal["error_attributes"]
    status_code: int
    exception: NotRequired[ExceptionSpec]


Scenario = ServiceFields | PostCallFailure | PostCallSuccess | ManagementFields | ErrorAttributes


class AuthenticationError(Exception):
    __module__ = "builtins"
    status_code: int = 401
    llm_provider: str = "openai"

    def __init__(self, message: str) -> None:
        super().__init__(f"litellm.AuthenticationError: {message}")


def exception_of(spec: ExceptionSpec) -> Exception:
    if spec["kind"] == "authentication":
        return AuthenticationError(spec["message"])
    return ValueError(spec["message"])


def with_carrier(kwargs: dict[str, JsonValue], parent: Span) -> CarriedKwargs:
    params = DICT.validate_python(kwargs["litellm_params"])
    metadata = DICT.validate_python(params["metadata"])
    carried_metadata: CarriedMetadata = {**metadata, "litellm_parent_otel_span": parent}
    carried_params: CarriedParams = {**params, "metadata": carried_metadata}
    return {**kwargs, "litellm_params": carried_params}


def failure_request(
    input_value: PostCallFailure, parent: Span | None
) -> tuple[CarriedKwargs, CarriedRequest]:
    request_data = DICT.validate_python(input_value["request_data"])
    bucket = "litellm_metadata" if "litellm_metadata" in request_data else "metadata"
    carried: CarriedMetadata = {**DICT.validate_python(request_data[bucket])}
    if parent is not None:
        carried["litellm_parent_otel_span"] = parent
    params = DICT.validate_python(input_value["kwargs"]["litellm_params"])
    carried_params: CarriedParams = {**params, "metadata": carried}
    kwargs: CarriedKwargs = {**input_value["kwargs"], "litellm_params": carried_params}
    return kwargs, {**request_data, bucket: carried}


async def service_call(
    logger: OpenTelemetry, input_value: ServiceFields, parent: Span | None, *, mode: Mode
) -> None:
    payload = ServiceLoggerPayload(
        is_error=mode == "failure", error=None, service=ServiceTypes(input_value["service"]),
        duration=1.0, call_type=input_value["call_type"], caller=input_value.get("caller"),
        event_metadata=None,
    )
    if mode == "success":
        await logger.async_service_success_hook(  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L792
            payload,
            parent,
            input_value["start_time"],
            input_value["end_time"],
            input_value.get("event_metadata"),
        )
        return
    await logger.async_service_failure_hook(  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L845
        payload=payload,
        error=input_value.get("error"),
        parent_otel_span=parent,
        start_time=input_value["start_time"],
        end_time=input_value["end_time"],
        event_metadata=input_value.get("event_metadata"),
    )


async def management_call(
    logger: OpenTelemetry, input_value: ManagementFields, parent: Span | None, *, mode: Mode
) -> None:
    exception = input_value.get("exception")
    payload = ManagementEndpointLoggingPayload(
        route=input_value["route"],
        request_data=DICT.validate_python(input_value["request_data"]),
        response=input_value.get("response"),
        exception=None if exception is None else exception_of(exception),
        start_time=datetime.fromtimestamp(input_value["start_time"], tz=UTC),
        end_time=datetime.fromtimestamp(input_value["end_time"], tz=UTC),
    )
    if mode == "success":
        await logger.async_management_endpoint_success_hook(  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L3382
            payload, parent
        )
        return
    await logger.async_management_endpoint_failure_hook(  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L3440
        payload, parent
    )


async def success_call(
    logger: OpenTelemetry,
    input_value: PostCallSuccess,
    carrier: Span | None,
    argument: Span | None,
) -> None:
    kwargs = input_value["kwargs"]
    details = kwargs if carrier is None else with_carrier(kwargs, carrier)
    logging_obj = Logging(
        model="gpt-4o", messages=None, stream=False, call_type="completion",
        start_time=datetime.fromtimestamp(input_value["start_time"], tz=UTC),
        litellm_call_id="call-1", function_id="fn-1",
    )
    logging_obj.model_call_details = details
    data: dict[str, Logging] = {"litellm_logging_obj": logging_obj}
    await logger.async_post_call_success_hook(  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L1030
        data, UserAPIKeyAuth(parent_otel_span=argument), ModelResponse(id="chatcmpl-b3858")
    )


async def failure_call(
    logger: OpenTelemetry, input_value: PostCallFailure, parent: Span | None
) -> None:
    kwargs, request_data = failure_request(input_value, parent)
    if input_value.get("after_failure_log") is True:
        start = datetime.fromtimestamp(input_value["start_time"], tz=UTC)
        end = datetime.fromtimestamp(input_value["end_time"], tz=UTC)
        await logger.async_log_failure_event(  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L765
            kwargs, None, start, end
        )
    team = input_value.get("team")
    key = UserAPIKeyAuth(parent_otel_span=parent)
    if team is not None:
        key = UserAPIKeyAuth(
            parent_otel_span=parent, team_id=team.get("id"), team_alias=team.get("alias")
        )
    await logger.async_post_call_failure_hook(  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L903
        request_data, exception_of(input_value["exception"]), key, input_value.get("traceback")
    )
