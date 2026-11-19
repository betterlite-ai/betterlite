from datetime import datetime, timezone
from typing import Literal, TypeAlias, TypedDict, cast

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

JsonObject: TypeAlias = dict[str, JsonValue]
START_TIME = datetime.fromtimestamp(100.25, tz=timezone.utc)


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


class ContentResponse:
    def __init__(self, body: str) -> None:
        self.content = body.encode("utf-8")

    def json(self) -> JsonValue:
        raise ValueError(self.content.decode("utf-8"))


class TextResponse:
    content: None = None

    def __init__(self, text: str) -> None:
        self.text = text

    def json(self) -> JsonValue:
        raise ValueError(self.text)


def response_of(arguments: JsonObject) -> BedrockGuardrailResponse:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L1253
        BedrockGuardrailResponse, arguments["response"]
    )


def response_wire(arguments: JsonObject) -> JsonObject:
    return TypeAdapter(JsonObject).validate_python(arguments["response"])


def request_data_of(arguments: JsonObject) -> JsonObject:
    return TypeAdapter(JsonObject).validate_python(arguments["request_data"])


def setup_of(arguments: JsonObject) -> GuardrailSetup:
    return TypeAdapter(GuardrailSetup).validate_python(arguments["guardrail"])


def chunk_usages(arguments: JsonObject) -> list[BedrockGuardrailUsage]:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L1290
        list[BedrockGuardrailUsage], arguments["completed_chunk_usages"]
    )


def region_of(arguments: JsonObject) -> str | None:
    return TypeAdapter(str | None).validate_python(arguments.get("aws_region_name"))


def build_guardrail(setup_value: GuardrailSetup) -> BedrockGuardrail:
    return BedrockGuardrail(
        guardrail_name=setup_value["name"],
        guardrailIdentifier=setup_value["identifier"],
        guardrailVersion=setup_value["version"],
        disable_exception_on_block=setup_value["disable_exception_on_block"],
    )


def bare_guardrail() -> BedrockGuardrail:
    return BedrockGuardrail()


def should_raise(arguments: JsonObject) -> JsonValue:
    return bare_guardrail()._should_raise_guardrail_blocked_exception(response_of(arguments))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2411


def blocked(arguments: JsonObject) -> JsonValue:
    blocks = bare_guardrail()._extract_blocked_assessments(response_of(arguments))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2243
    return TypeAdapter(JsonValue).validate_python(blocks)


def violation_names(arguments: JsonObject) -> JsonValue:
    names = bare_guardrail()._extract_violation_category_names(response_of(arguments))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2217
    return TypeAdapter(JsonValue).validate_python(names)


def view_response(view: ResponseView) -> httpx.Response | ContentResponse | TextResponse:
    if view["kind"] == "json":
        return httpx.Response(200, json=view["payload"])
    if view["kind"] == "content":
        return ContentResponse(view["body"])
    return TextResponse(view["text"])


def has_exception(arguments: JsonObject) -> JsonValue:
    view = TypeAdapter(ResponseView).validate_python(arguments["view"])
    return bare_guardrail()._check_bedrock_response_for_exception(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2092
        view_response(view)
    )


def bedrock_status(arguments: JsonObject) -> JsonValue:
    status_code = TypeAdapter(int).validate_python(arguments["status_code"])
    body = TypeAdapter(str).validate_python(arguments["body"])
    response = httpx.Response(status_code, content=body.encode("utf-8"))
    return bare_guardrail()._get_bedrock_guardrail_response_status(response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2125


def tracing(arguments: JsonObject) -> JsonValue:
    detail = bare_guardrail()._build_tracing_detail(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2176
        response_of(arguments), region_of(arguments)
    )
    return TypeAdapter(JsonValue).validate_python(detail)


def modify_outcome(error: ModifyResponseException) -> JsonValue:
    request_data = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L1201
        JsonObject, error.request_data
    )
    detection_info = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L1201
        JsonObject, error.detection_info
    )
    return TypeAdapter(JsonValue).validate_python(
        {
            "kind": "modify",
            "message": error.message,
            "model": error.model,
            "request_data": request_data,
            "guardrail_name": error.guardrail_name,
            "detection_info": detection_info,
        }
    )


def http_outcome(error: HTTPException) -> JsonValue:
    detail = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2409
        JsonValue, error.detail
    )
    return TypeAdapter(JsonValue).validate_python(
        {"kind": "http", "status": error.status_code, "detail": detail}
    )


def http_exception(arguments: JsonObject) -> JsonValue:
    guardrail = build_guardrail(setup_of(arguments))
    outcome = guardrail._get_http_exception_for_blocked_guardrail(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L2374
        response_of(arguments), request_data_of(arguments)
    )
    match outcome:
        case ModifyResponseException():
            return modify_outcome(outcome)
        case HTTPException():
            return http_outcome(outcome)


def log_attempt(arguments: JsonObject) -> JsonValue:
    guardrail = build_guardrail(setup_of(arguments))
    request_data = request_data_of(arguments)
    status_code = TypeAdapter(int).validate_python(arguments["status_code"])
    body = TypeAdapter(str).validate_python(arguments["body"])
    guardrail._log_apply_guardrail_attempt(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L1280
        httpx.Response(status_code, content=body.encode("utf-8")),
        response_wire(arguments),
        request_data,
        GuardrailEventHooks.during_call,
        START_TIME,
        region_of(arguments),
        chunk_usages(arguments),
    )
    return request_data


def log_success(arguments: JsonObject) -> JsonValue:
    guardrail = build_guardrail(setup_of(arguments))
    request_data = request_data_of(arguments)
    guardrail._log_apply_guardrail_success(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L1321
        response_of(arguments),
        request_data,
        GuardrailEventHooks.during_call,
        START_TIME,
        region_of(arguments),
    )
    return request_data


def log_failure(arguments: JsonObject) -> JsonValue:
    guardrail = build_guardrail(setup_of(arguments))
    request_data = request_data_of(arguments)
    detail = TypeAdapter(str).validate_python(arguments["detail"])
    guardrail._log_apply_guardrail_failure(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L1355
        detail,
        request_data,
        GuardrailEventHooks.during_call,
        START_TIME,
        region_of(arguments),
        chunk_usages(arguments),
    )
    return request_data


def assessment_view(operation: str, arguments: JsonObject) -> JsonValue:
    if operation == "should_raise":
        return should_raise(arguments)
    if operation == "blocked_assessments":
        return blocked(arguments)
    if operation == "violation_names":
        return violation_names(arguments)
    raise ValueError(operation)


def decision_view(operation: str, arguments: JsonObject) -> JsonValue:
    if operation == "has_exception":
        return has_exception(arguments)
    if operation == "status":
        return bedrock_status(arguments)
    if operation == "tracing_detail":
        return tracing(arguments)
    if operation == "http_exception":
        return http_exception(arguments)
    raise ValueError(operation)


def logging_view(operation: str, arguments: JsonObject) -> JsonValue:
    if operation == "log_attempt":
        return log_attempt(arguments)
    if operation == "log_success":
        return log_success(arguments)
    if operation == "log_failure":
        return log_failure(arguments)
    raise ValueError(operation)


def run(input_value: Input) -> JsonValue:
    ((operation, arguments),) = input_value["bedrock_decisions"].items()
    if operation in {"should_raise", "blocked_assessments", "violation_names"}:
        return assessment_view(operation, arguments)
    if operation in {"has_exception", "status", "tracing_detail", "http_exception"}:
        return decision_view(operation, arguments)
    return logging_view(operation, arguments)
