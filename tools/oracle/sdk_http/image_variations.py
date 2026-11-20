import asyncio
from typing import Literal, Protocol, TypedDict, Unpack, cast

import litellm.images.main as images_main
import openai
from litellm.types.utils import ImageResponse
from openai.types.image import Image as OpenAIImage
from pydantic import BaseModel, ConfigDict, Field

from openai_chat import Failure, optional_text, status_code


class VariationParams(TypedDict):
    image: tuple[str, bytes, str | None]
    model: str
    n: int
    response_format: Literal["url", "b64_json"]
    size: str | None
    user: str | None
    api_base: str
    api_key: str | None
    timeout: float | None
    custom_llm_provider: str | None


class ImageVariationEntry(Protocol):
    def image_variation(self, /, **kwargs: Unpack[VariationParams]) -> ImageResponse: ...

    async def aimage_variation(self, /, **kwargs: Unpack[VariationParams]) -> ImageResponse: ...


ENTRY = cast("ImageVariationEntry", images_main)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674


class Image(BaseModel):
    model_config = ConfigDict(extra="forbid")

    filename: str
    content: str
    content_type: str | None = None


class Params(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_key: str | None = None
    custom_llm_provider: str | None = None
    timeout_seconds: float | None = None


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    image: Image
    model: str | None = None
    n: int | None = None
    response_format: Literal["url", "b64_json"] | None = None
    size: str | None = None
    user: str | None = None
    asynchronous: bool = False
    litellm_params: Params = Field(default_factory=Params)


class Datum(BaseModel):
    b64_json: str | None = None
    revised_prompt: str | None = None
    url: str | None = None


class Success(BaseModel):
    created: int
    data: list[Datum]


class Outcome(BaseModel):
    success: Success | None = None
    failure: Failure | None = None


class ImageVariationsValue(BaseModel):
    value: Outcome


class Result(BaseModel):
    image_variations: ImageVariationsValue


class ImageVariationsRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    image_variations: Call

    def serialize(self, api_base: str) -> str:
        return Result(
            image_variations=ImageVariationsValue(
                value=run_variation(self.image_variations, api_base)
            )
        ).model_dump_json(by_alias=True, exclude_unset=True)


def api_failure(error: openai.APIError) -> Failure:
    fields: dict[str, str | int | None] = {
        "cls": type(error).__name__,
        "status": status_code(error),
        "message": error.message,
    }
    for name in ("type", "param", "code"):
        fields.update(optional_text(error, name))
    return Failure.model_validate(fields)


def datum(image: OpenAIImage) -> Datum:
    fields: dict[str, str] = {}
    if image.url is not None:
        fields["url"] = image.url
    if image.b64_json is not None:
        fields["b64_json"] = image.b64_json
    if image.revised_prompt is not None:
        fields["revised_prompt"] = image.revised_prompt
    return Datum.model_validate(fields)


def project_success(response: ImageResponse) -> Outcome:
    data = response.data
    if data is None:
        data = []
    return Outcome(
        success=Success(
            created=response.created,
            data=[datum(image) for image in data],
        )
    )


def run_variation(call: Call, api_base: str) -> Outcome:
    image = (
        call.image.filename,
        call.image.content.encode(),
        call.image.content_type,
    )
    model = call.model if call.model is not None else "dall-e-2"
    n = call.n if call.n is not None else 1
    response_format = call.response_format if call.response_format is not None else "url"
    try:
        if call.asynchronous:
            response = asyncio.run(
                ENTRY.aimage_variation(
                    image=image,
                    model=model,
                    n=n,
                    response_format=response_format,
                    size=call.size,
                    user=call.user,
                    api_base=api_base,
                    api_key=call.litellm_params.api_key,
                    timeout=call.litellm_params.timeout_seconds,
                    custom_llm_provider=call.litellm_params.custom_llm_provider,
                )
            )
        else:
            response = ENTRY.image_variation(
                image=image,
                model=model,
                n=n,
                response_format=response_format,
                size=call.size,
                user=call.user,
                api_base=api_base,
                api_key=call.litellm_params.api_key,
                timeout=call.litellm_params.timeout_seconds,
                custom_llm_provider=call.litellm_params.custom_llm_provider,
            )
    except ValueError as error:
        return Outcome(failure=Failure(cls="ValueError", status=0, message=str(error)))
    except openai.APIError as error:
        return Outcome(failure=api_failure(error))
    return project_success(response)
