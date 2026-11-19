from abc import ABC, abstractmethod
from collections.abc import Callable, Mapping
from datetime import UTC, datetime
from typing import Final, Literal, TypeAlias, TypedDict, cast

import httpx
from fastapi import HTTPException
from litellm.exceptions import ModifyResponseException
from litellm.proxy.guardrails.guardrail_hooks.bedrock_guardrails import BedrockGuardrail
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.proxy.guardrails.guardrail_hooks.bedrock_guardrails import (
    BedrockGuardrailResponse,
    BedrockGuardrailUsage,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JsonObject: TypeAlias = dict[str, JsonValue]
START_TIME = datetime.fromtimestamp(100.25, tz=UTC)


class Input(TypedDict):
    bedrock_decisions: dict[str, JsonObject]


class GuardrailSetup(TypedDict):
    name: str | None
    identifier: str | None
    version: str | None
    disable_exception_on_block: bool


class JsonView(TypedDict):
    kind: Literal["json"]
    payload: JsonValue


class ContentView(TypedDict):
    kind: Literal["content"]
    body: str


class TextView(TypedDict):
    kind: Literal["text"]
    text: str


ResponseView: TypeAlias = JsonView | ContentView | TextView


class ModifyView(TypedDict):
    kind: Literal["modify"]
    message: str
    model: str
    request_data: JsonObject
    guardrail_name: str | None
    detection_info: JsonObject


class HttpView(TypedDict):
    kind: Literal["http"]
    status: int
    detail: JsonValue


# httpx.Response parses its body in json(), so a stand-in whose json() fails must be its own type
class FailingJsonResponse(ABC):
    content: bytes | None
    text: str

    @abstractmethod
    def json(self) -> JsonValue:
        ...


class ContentResponse(FailingJsonResponse):
    def __init__(self, body: str) -> None:
        self.content = body.encode("utf-8")
        self.text = body

    @override
    def json(self) -> JsonValue:
        raise ValueError(self.text)


class TextResponse(FailingJsonResponse):
    def __init__(self, text: str) -> None:
        self.content = None
        self.text = text

    @override
    def json(self) -> JsonValue:
        raise ValueError(self.text)


def response_of(arguments: JsonObject) -> BedrockGuardrailResponse:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L1253
        BedrockGuardrailResponse, arguments["response"]
    )


def chunk_usages(arguments: JsonObject) -> list[BedrockGuardrailUsage]:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L1290
        list[BedrockGuardrailUsage], arguments["completed_chunk_usages"]
    )


def configured(arguments: JsonObject) -> tuple[BedrockGuardrail, JsonObject]:
    setup = TypeAdapter(GuardrailSetup).validate_python(arguments["guardrail"])
    request_data = TypeAdapter(JsonObject).validate_python(arguments["request_data"])
    guardrail = BedrockGuardrail(
        guardrail_name=setup["name"],
        guardrailIdentifier=setup["identifier"],
        guardrailVersion=setup["version"],
        disable_exception_on_block=setup["disable_exception_on_block"],
    )
    return guardrail, request_data


def view_response(view: ResponseView) -> httpx.Response | ContentResponse | TextResponse:
    if view["kind"] == "json":
        return httpx.Response(200, json=view["payload"])
    if view["kind"] == "content":
        return ContentResponse(view["body"])
    return TextResponse(view["text"])


def should_raise(arguments: JsonObject) -> JsonValue:
    response = response_of(arguments)
    return BedrockGuardrail()._should_raise_guardrail_blocked_exception(response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2411


def blocked(arguments: JsonObject) -> JsonValue:
    return TypeAdapter(JsonValue).validate_python(
        cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2243
            JsonValue,
            BedrockGuardrail()._extract_blocked_assessments(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2243
                response_of(arguments)
            ),
        )
    )


def violation_names(arguments: JsonObject) -> JsonValue:
    names = BedrockGuardrail()._extract_violation_category_names(response_of(arguments))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2217
    return TypeAdapter(JsonValue).validate_python(names)


def has_exception(arguments: JsonObject) -> JsonValue:
    view = TypeAdapter(ResponseView).validate_python(arguments["view"])
    return BedrockGuardrail()._check_bedrock_response_for_exception(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2092
        view_response(view)
    )


def bedrock_status(arguments: JsonObject) -> JsonValue:
    status_code = TypeAdapter(int).validate_python(arguments["status_code"])
    body = TypeAdapter(str).validate_python(arguments["body"])
    response = httpx.Response(status_code, content=body.encode("utf-8"))
    return BedrockGuardrail()._get_bedrock_guardrail_response_status(response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2125


def tracing(arguments: JsonObject) -> JsonValue:
    detail = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2176
        JsonValue,
        BedrockGuardrail()._build_tracing_detail(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2176
            response_of(arguments),
            TypeAdapter(str | None).validate_python(arguments.get("aws_region_name")),
        ),
    )
    return TypeAdapter(JsonValue).validate_python(detail)


def modify_outcome(error: ModifyResponseException) -> JsonValue:
    request_data = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L1225
        JsonObject, error.request_data
    )
    detection_info = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L1227
        JsonObject, error.detection_info
    )
    view: ModifyView = {
        "kind": "modify",
        "message": error.message,
        "model": error.model,
        "request_data": request_data,
        "guardrail_name": error.guardrail_name,
        "detection_info": detection_info,
    }
    return TypeAdapter(JsonValue).validate_python(view)


def http_outcome(error: HTTPException) -> JsonValue:
    detail = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2409
        JsonValue, error.detail
    )
    view: HttpView = {
        "kind": "http",
        "status": error.status_code,
        "detail": detail,
    }
    return TypeAdapter(JsonValue).validate_python(view)


def http_exception(arguments: JsonObject) -> JsonValue:
    guardrail, request_data = configured(arguments)
    outcome = guardrail._get_http_exception_for_blocked_guardrail(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2374
        response_of(arguments), request_data
    )
    match outcome:
        case ModifyResponseException():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L1201
            return modify_outcome(outcome)
        case HTTPException():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2374
            return http_outcome(outcome)


def log_attempt(arguments: JsonObject) -> JsonValue:
    guardrail, request_data = configured(arguments)
    status_code = TypeAdapter(int).validate_python(arguments["status_code"])
    body = TypeAdapter(str).validate_python(arguments["body"])
    region = TypeAdapter(str | None).validate_python(arguments.get("aws_region_name"))
    response = httpx.Response(status_code, content=body.encode("utf-8"))
    json_response = TypeAdapter(JsonObject).validate_python(arguments["response"])
    guardrail._log_apply_guardrail_attempt(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L1280
        response,
        json_response,
        request_data,
        GuardrailEventHooks.during_call,
        START_TIME,
        region,
        chunk_usages(arguments),
    )
    return request_data


def log_success(arguments: JsonObject) -> JsonValue:
    guardrail, request_data = configured(arguments)
    response = response_of(arguments)
    region = TypeAdapter(str | None).validate_python(arguments.get("aws_region_name"))
    guardrail._log_apply_guardrail_success(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L1321
        response, request_data, GuardrailEventHooks.during_call, START_TIME, region
    )
    return request_data


def log_failure(arguments: JsonObject) -> JsonValue:
    guardrail, request_data = configured(arguments)
    detail = TypeAdapter(str).validate_python(arguments["detail"])
    guardrail._log_apply_guardrail_failure(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L1355
        detail,
        request_data,
        GuardrailEventHooks.during_call,
        START_TIME,
        TypeAdapter(str | None).validate_python(arguments.get("aws_region_name")),
        chunk_usages(arguments),
    )
    return request_data


OPERATIONS: Final[Mapping[str, Callable[[JsonObject], JsonValue]]] = {
    "should_raise": should_raise,
    "blocked_assessments": blocked,
    "violation_names": violation_names,
    "has_exception": has_exception,
    "status": bedrock_status,
    "tracing_detail": tracing,
    "http_exception": http_exception,
    "log_attempt": log_attempt,
    "log_success": log_success,
    "log_failure": log_failure,
}


def run(input_value: Input) -> JsonValue:
    ((operation, arguments),) = input_value["bedrock_decisions"].items()
    return OPERATIONS[operation](arguments)
