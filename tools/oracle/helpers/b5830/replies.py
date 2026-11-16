from typing import Final, TypedDict

from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

from .inputs import Object


class ImageDatum(TypedDict):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None


class TokenDetails(TypedDict):
    image_tokens: int
    text_tokens: int


class UsageFields(TypedDict):
    input_tokens: int
    input_tokens_details: TokenDetails
    output_tokens: int
    total_tokens: int


class ImageReply(TypedDict):
    created: int
    data: list[ImageDatum]
    usage: UsageFields | None


class HiddenFields(TypedDict, total=False):
    additional_headers: dict[str, JsonValue]
    response_cost_details: dict[str, JsonValue]
    model: JsonValue


class Hidden(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    params: HiddenFields = Field(alias="_hidden_params")


REPLY: Final[TypeAdapter[ImageReply]] = TypeAdapter(ImageReply)


def image_values(data: list[ImageDatum]) -> list[JsonValue]:
    values: list[JsonValue] = []
    for datum in data:
        fields: Object = {}
        if datum["b64_json"] is not None:
            fields["b64_json"] = datum["b64_json"]
        if datum["url"] is not None:
            fields["url"] = datum["url"]
        if datum["revised_prompt"] is not None:
            fields["revised_prompt"] = datum["revised_prompt"]
        values.append(fields)
    return values


def usage_fields(usage: UsageFields) -> Object:
    details = usage["input_tokens_details"]
    token_details: TokenDetails = {
        "image_tokens": details["image_tokens"],
        "text_tokens": details["text_tokens"],
    }
    return {
        "input_tokens": usage["input_tokens"],
        "input_tokens_details": token_details,
        "output_tokens": usage["output_tokens"],
        "total_tokens": usage["total_tokens"],
    }


def hidden_fields(reply: ImageResponse) -> Object:
    params = Hidden.model_validate(reply).params
    fields: Object = {}
    headers = params.get("additional_headers")
    if headers is not None:
        fields["additional_headers"] = headers
    details = params.get("response_cost_details")
    if details is not None:
        fields["response_cost_details"] = details
    fields["model"] = params.get("model")
    return fields


def reply_usage(parsed: ImageReply) -> Object | None:
    usage = parsed["usage"]
    if usage is None:
        return None
    return usage_fields(usage)


def reply_fields(parsed: ImageReply, reply: ImageResponse) -> Object:
    return {
        "data": image_values(parsed["data"]),
        "usage": reply_usage(parsed),
        "hidden": hidden_fields(reply),
    }


def generation_fields(reply: ImageResponse) -> Object:
    parsed = REPLY.validate_json(reply.model_dump_json())
    fields = reply_fields(parsed, reply)
    fields["created"] = parsed["created"]
    return fields


def edit_fields(reply: ImageResponse) -> Object:
    return reply_fields(REPLY.validate_json(reply.model_dump_json()), reply)
