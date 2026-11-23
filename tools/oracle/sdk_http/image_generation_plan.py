import asyncio
from typing import Literal, Protocol

import litellm.images.main as image_main
import openai
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

from image_generation_call import Call as ImageGenerationCall
from image_generation_call import Outcome, apply_settings, call_kwargs, resolved_settings
from image_generation_call import run as run_image_generation
from image_variations import api_failure

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
IMAGE: TypeAdapter[ImageResponse] = TypeAdapter(ImageResponse)


# litellm's client decorator leaves aimage_generation untyped; this protocol carries its shape
class AsyncImageEntry(Protocol):
    async def aimage_generation(self, /, **kwargs: JsonValue) -> ImageResponse: ...


ENTRY: AsyncImageEntry = image_main


class ImageGenerationPlanCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    sync_mode: Literal["sync", "async"]
    call: ImageGenerationCall


class ImageGenerationPlanValue(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: Outcome


class ImageGenerationPlanOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    image_generation_plan: ImageGenerationPlanValue


def async_outcome(call: ImageGenerationCall, api_base: str) -> Outcome:
    try:
        response = asyncio.run(ENTRY.aimage_generation(**call_kwargs(call, api_base)))
    except openai.APIError as error:
        return Outcome(failure=api_failure(error))
    parsed = IMAGE.validate_python(response)
    return Outcome(result=JSON.validate_json(parsed.model_dump_json(exclude_none=True)))


def run(call: ImageGenerationPlanCall, api_base: str) -> Outcome:
    apply_settings(resolved_settings(call.call, api_base), api_base)
    if call.sync_mode == "sync":
        return run_image_generation(call.call, api_base)
    return async_outcome(call.call, api_base)


class ImageGenerationPlanRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    image_generation_plan: ImageGenerationPlanCall

    def serialize(self, api_base: str) -> str:
        value = ImageGenerationPlanValue(value=run(self.image_generation_plan, api_base))
        envelope = ImageGenerationPlanOutcome(image_generation_plan=value)
        return envelope.model_dump_json(by_alias=True, exclude_unset=True)
