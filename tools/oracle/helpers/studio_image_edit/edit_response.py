import json
from typing import Final, TypedDict

import httpx
from pydantic import JsonValue, TypeAdapter

from studio_image_edit.edit_config import CONFIG, EDIT_MODEL
from studio_image_edit.edit_values import JsonObject


class ResponseCase(TypedDict):
    body: JsonObject


class EditDatum(TypedDict):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None
    provider_specific_fields: dict[str, JsonValue] | None


class TokenDetails(TypedDict):
    image_tokens: int
    text_tokens: int


class UsageFields(TypedDict):
    input_tokens: int
    input_tokens_details: TokenDetails
    output_tokens: int
    total_tokens: int
    prompt_tokens: int
    prompt_tokens_details: TokenDetails
    completion_tokens: int
    completion_tokens_details: TokenDetails
    output_tokens_details: TokenDetails


class EditReply(TypedDict):
    data: list[EditDatum]
    usage: UsageFields | None


REPLY: Final[TypeAdapter[EditReply]] = TypeAdapter(EditReply)


def detail_value(details: TokenDetails) -> JsonObject:
    return {
        "image_tokens": details["image_tokens"],
        "text_tokens": details["text_tokens"],
    }


def usage_value(usage: UsageFields) -> JsonObject:
    return {
        "input_tokens": usage["input_tokens"],
        "input_tokens_details": detail_value(usage["input_tokens_details"]),
        "output_tokens": usage["output_tokens"],
        "total_tokens": usage["total_tokens"],
        "prompt_tokens": usage["prompt_tokens"],
        "prompt_tokens_details": detail_value(usage["prompt_tokens_details"]),
        "completion_tokens": usage["completion_tokens"],
        "completion_tokens_details": detail_value(usage["completion_tokens_details"]),
        "output_tokens_details": detail_value(usage["output_tokens_details"]),
    }


def image_value(datum: EditDatum) -> JsonObject:
    fields = datum["provider_specific_fields"]
    signature = None if fields is None else fields.get("thought_signature")
    return {"b64_json": datum["b64_json"], "thought_signature": signature}


def response_value(case: ResponseCase) -> JsonObject:
    raw = httpx.Response(200, content=json.dumps(case["body"]))
    finished = CONFIG.transform_image_edit_response(EDIT_MODEL, raw, None)
    # the transform builds its own ImageResponse, whose constructor seeds a zero usage
    if "usageMetadata" not in case["body"]:
        finished.usage = None
    parsed = REPLY.validate_json(finished.model_dump_json())
    usage = parsed["usage"]
    return {
        "images": [image_value(datum) for datum in parsed["data"]],
        "usage": None if usage is None else usage_value(usage),
    }
