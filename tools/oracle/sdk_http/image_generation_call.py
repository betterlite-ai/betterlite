from typing import Final

import httpx
import litellm
import openai
from litellm.exceptions import RateLimitError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.custom_llm import CustomLLM
from litellm.types.llms.custom_llm import CustomLLMItem
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

from image_variations import api_failure
from openai_chat import Failure

MOCK: Final = "{mock}"
FIRST_ERROR_STATUS: Final = 400
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
IMAGE: TypeAdapter[ImageResponse] = TypeAdapter(ImageResponse)


class Settings(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_base: str | None = None
    api_version: str | None = None
    api_key: str | None = None


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str | None = None
    prompt: str
    custom_llm_provider: str | None = None
    n: int | None = None
    quality: str | None = None
    response_format: str | None = None
    size: str | None = None
    style: str | None = None
    user: str | None = None
    timeout: float | None = None
    api_key: str | None = None
    api_version: str | None = None
    aimg_generation: bool = False
    mock_response: str | None = None
    headers: dict[str, str] = Field(default_factory=dict)
    extra_headers: dict[str, str] | None = None
    organization: str | None = None
    base_model: str | None = None
    custom_providers: list[str] = Field(default_factory=list)
    litellm_settings: Settings = Field(default_factory=Settings)


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    result: JsonValue = None
    failure: Failure | None = None


class TransportValue(BaseModel):
    value: Outcome


class Envelope(BaseModel):
    image_generation_call: TransportValue


class CustomHandler(CustomLLM):
    def __init__(self, origin: str) -> None:
        super().__init__()
        self.origin = origin

    @override
    def image_generation(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L526-L547
        self,
        model: str,
        prompt: str,
        api_key: str | None,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L526-L547
        api_base: str | None,
        model_response: ImageResponse,
        optional_params: dict[str, JsonValue],  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L526-L547
        logging_obj: Logging,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L526-L547
        timeout: float | httpx.Timeout | None = None,
        client: HTTPHandler | None = None,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L526-L547
    ) -> ImageResponse:
        base = self.origin if api_base is None else api_base
        body: dict[str, JsonValue] = {"model": model, "prompt": prompt}
        response = httpx.post(
            f"{base}/custom/images",
            json=body,
            timeout=timeout,
        )
        if response.status_code >= FIRST_ERROR_STATUS:
            raise RateLimitError(
                message=f"provider answered {response.status_code}",
                llm_provider="custom",
                model=model,
            )
        parsed = IMAGE.validate_json(response.content)
        model_response.created = parsed.created
        model_response.data = parsed.data
        model_response.usage = parsed.usage
        return model_response


def apply_settings(settings: Settings, api_base: str) -> None:
    litellm.suppress_debug_info = True
    litellm.api_key = settings.api_key
    base = settings.api_base
    litellm.api_base = None if base is None else base.replace(MOCK, api_base)
    litellm.api_version = settings.api_version


def install_custom_providers(call: Call, api_base: str) -> None:
    entries: list[CustomLLMItem] = [
        {"provider": provider, "custom_handler": CustomHandler(api_base)}
        for provider in call.custom_providers
    ]
    litellm.custom_provider_map = entries  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L503-L506


def call_kwargs(call: Call, api_base: str) -> dict[str, JsonValue]:
    options: dict[str, JsonValue] = {
        "model": call.model,
        "custom_llm_provider": call.custom_llm_provider,
        "n": call.n,
        "quality": call.quality,
        "response_format": call.response_format,
        "size": call.size,
        "style": call.style,
        "user": call.user,
        "timeout": call.timeout,
        "api_key": call.api_key,
        "api_version": call.api_version,
        "mock_response": call.mock_response,
        "organization": call.organization,
        "base_model": call.base_model,
    }
    kwargs: dict[str, JsonValue] = {
        "prompt": call.prompt,
        "api_base": api_base,
        **{name: value for name, value in options.items() if value is not None},
    }
    if call.aimg_generation:
        kwargs["aimg_generation"] = True
    if call.headers:
        kwargs["headers"] = JSON.validate_python(call.headers)
    if call.extra_headers is not None:
        kwargs["extra_headers"] = JSON.validate_python(call.extra_headers)
    return kwargs


def resolved_settings(call: Call, api_base: str) -> Settings:
    settings = call.litellm_settings
    base = settings.api_base
    return Settings(
        api_base=None if base is None else base.replace(MOCK, api_base),
        api_version=settings.api_version,
        api_key=settings.api_key,
    )


def run(call: Call, api_base: str) -> Outcome:
    apply_settings(resolved_settings(call, api_base), api_base)
    install_custom_providers(call, api_base)
    try:
        response: ImageResponse = litellm.image_generation(**call_kwargs(call, api_base))  # type: ignore[call-overload]  # pyright: ignore[reportAny, reportArgumentType, reportCallIssue, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L183-L561
    except openai.APIError as error:
        return Outcome(failure=api_failure(error))
    finally:
        litellm.custom_provider_map = []  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L503-L506
    parsed = IMAGE.validate_python(response)
    return Outcome(result=JSON.validate_json(parsed.model_dump_json(exclude_none=True)))


class ImageGenerationCallRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    image_generation_call: Call

    def serialize(self, api_base: str) -> str:
        value = TransportValue(value=run(self.image_generation_call, api_base))
        envelope = Envelope(image_generation_call=value)
        return envelope.model_dump_json(by_alias=True, exclude_unset=True)
