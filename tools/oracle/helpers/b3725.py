import asyncio
from typing import Final, Literal, TypedDict, cast

import httpx
from litellm.exceptions import ModifyResponseException
from litellm.proxy.guardrails.guardrail_hooks.bedrock_guardrails import BedrockGuardrail
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
DICT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
LIST: TypeAdapter[list[JsonValue]] = TypeAdapter(list[JsonValue])
GUARDRAIL_NAME = "bedrock-checks"
SIGNING: Final = {
    "aws_access_key_id": "AKIAIOSFODNN7EXAMPLE",
    "aws_secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    "aws_region_name": "us-west-2",
}
PROJECTED = (
    "guardrail_name",
    "guardrail_provider",
    "guardrail_mode",
    "guardrail_status",
    "guardrail_response",
    "violation_categories",
    "guardrail_action",
)


class Reply(TypedDict):
    status: int
    body: JsonValue
    raw: str | None


class Checks(TypedDict):
    source: Literal["INPUT", "OUTPUT"]
    messages: list[dict[str, JsonValue]] | None
    response: dict[str, JsonValue] | None
    logging_event_type: Literal["pre_call", "post_call", "during_call"] | None
    checks: dict[str, JsonValue]
    content_filter_threshold: float | None
    prompt_attack_threshold: float | None
    pii_confidence_threshold: float | None
    disable_exception_on_block: bool
    data: dict[str, JsonValue]
    reply: Reply


class Captured(TypedDict):
    url: str
    authorization: str | None
    content_type: str | None
    body: JsonValue


class ScriptedReplies:
    def __init__(self, reply: Reply, captured: list[Captured]) -> None:
        self.reply = reply
        self.captured = captured

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.captured.append(capture(request))
        if self.reply["raw"] is not None:
            return httpx.Response(self.reply["status"], content=self.reply["raw"])
        return httpx.Response(self.reply["status"], json=self.reply["body"])


def capture(request: httpx.Request) -> Captured:
    headers: dict[str, str] = dict(request.headers.items())
    return {
        "url": str(request.url),
        "authorization": headers.get("authorization"),
        "content_type": headers.get("content-type"),
        "body": JSON.validate_json(request.content),
    }


def build_guardrail(input_value: Checks) -> BedrockGuardrail:
    return BedrockGuardrail(
        guardrail_name=GUARDRAIL_NAME,
        checks=input_value["checks"],
        content_filter_threshold=input_value["content_filter_threshold"],
        prompt_attack_threshold=input_value["prompt_attack_threshold"],
        pii_confidence_threshold=input_value["pii_confidence_threshold"],
        disable_exception_on_block=input_value["disable_exception_on_block"],
        aws_access_key_id=SIGNING["aws_access_key_id"],
        aws_secret_access_key=SIGNING["aws_secret_access_key"],
        aws_region_name=SIGNING["aws_region_name"],
    )


def response_of(value: dict[str, JsonValue] | None) -> ModelResponse | None:
    if value is None:
        return None
    return ModelResponse.model_validate(value)


def event_of(input_value: Checks) -> GuardrailEventHooks | None:
    configured = input_value["logging_event_type"]
    if configured is None:
        return None
    return GuardrailEventHooks(configured)


async def invoke(
    guardrail: BedrockGuardrail, input_value: Checks, data: dict[str, JsonValue]
) -> JsonValue:
    try:
        result = await guardrail.make_bedrock_api_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/bedrock_guardrails.py#L863-L870
            source=input_value["source"],
            messages=cast("list[AllMessageValues] | None", input_value["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            response=response_of(input_value["response"]),
            request_data=data,
            logging_event_type=event_of(input_value),
        )
    except HTTPException as error:
        return {
            "kind": "rejected",
            "status_code": error.status_code,
            "detail": JSON.validate_python(error.detail),
        }
    except ModifyResponseException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L1201-L1233
        return {"kind": "modify", "message": error.message, "model": error.model}
    return {"kind": "allowed", "result": JSON.validate_python(result)}


def records_of(data: dict[str, JsonValue]) -> list[JsonValue]:
    metadata = data.get("metadata")
    if metadata is None:
        return []
    bucket = DICT.validate_python(metadata)
    stored = bucket.get("standard_logging_guardrail_information")
    if stored is None:
        return []
    return LIST.validate_python(stored)


def project_record(value: JsonValue) -> dict[str, JsonValue]:
    record = DICT.validate_python(value)
    return {key: record.get(key) for key in PROJECTED}


def run(input_value: Checks) -> JsonValue:
    guardrail = build_guardrail(input_value)
    captured: list[Captured] = []
    handler = ScriptedReplies(input_value["reply"], captured)
    guardrail.async_handler.client = httpx.AsyncClient(transport=httpx.MockTransport(handler))
    data: dict[str, JsonValue] = dict(input_value["data"])
    outcome = asyncio.run(invoke(guardrail, input_value, data))
    records = [project_record(record) for record in records_of(data)]
    return {
        "outcomes": [outcome],
        "records": JSON.validate_python(records),
        "requests": JSON.validate_python(captured),
    }
