from typing import Final

import httpx
import litellm
from litellm.exceptions import APIError, RateLimitError
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

MOCK: Final = "{mock}"
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
IMAGE: TypeAdapter[ImageResponse] = TypeAdapter(ImageResponse)
STRINGS: TypeAdapter[dict[str, str]] = TypeAdapter(dict[str, str])


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


class Failure(BaseModel):
    model_config = ConfigDict(extra="forbid")

    class_name: str = Field(serialization_alias="class")
    status: int
    kind: str | None = Field(default=None, serialization_alias="type")
    param: str | None = None
    code: str | None = None
    message: str


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    result: JsonValue = None
    failure: Failure | None = None


class TransportValue(BaseModel):
    value: Outcome


class Envelope(BaseModel):
    image_generation_call: TransportValue


class CustomHandler:
    def __init__(self, origin: str) -> None:
        self.origin = origin

    def image_generation(
        self,
        model: str,
        prompt: str,
        api_key: str | None,
        api_base: str | None,
        model_response: ImageResponse,
        optional_params: dict[str, JsonValue],
        logging_obj: object,
        timeout: float,
        client: object,
    ) -> ImageResponse:
        base = self.origin if api_base is None else api_base
        response = httpx.post(
            f"{base}/custom/images",
            json={"model": model, "prompt": prompt},
            timeout=timeout,
        )
        if response.status_code >= 400:
            raise RateLimitError(
                message=f"provider answered {response.status_code}",
                llm_provider="custom",
                model=model,
            )
        return IMAGE.validate_json(response.content)


def substituted(value: str, api_base: str) -> str:
    return value.replace(MOCK, api_base)


def apply_settings(settings: Settings, api_base: str) -> None:
    litellm.suppress_debug_info = True
    litellm.api_key = settings.api_key
    litellm.api_base = None if settings.api_base is None else substituted(settings.api_base, api_base)
    litellm.api_version = settings.api_version


def install_custom_providers(call: Call, api_base: str) -> None:
    map: list[dict[str, object]] = [
        {"provider": provider, "custom_handler": CustomHandler(api_base)}
        for provider in call.custom_providers
    ]
    litellm.custom_provider_map = map  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L503-L506


def error_failure(error: APIError) -> Failure:
    return Failure(
        class_name=type(error).__name__,
        status=error.status_code,
        kind=error.type,  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L853-L861
        param=error.param,
        code=error.code,
        message=error.message,
    )


def call_kwargs(call: Call, api_base: str) -> dict[str, JsonValue]:
    kwargs: dict[str, JsonValue] = {
        "prompt": call.prompt,
        "api_base": api_base,
    }
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
    for name, value in options.items():
        if value is not None:
            kwargs[name] = value
    if call.aimg_generation:
        kwargs["aimg_generation"] = True
    if call.headers:
        kwargs["headers"] = STRINGS.validate_python(call.headers)
    if call.extra_headers is not None:
        kwargs["extra_headers"] = STRINGS.validate_python(call.extra_headers)
    return kwargs


def resolved_settings(call: Call, api_base: str) -> Settings:
    settings = call.litellm_settings
    return Settings(
        api_base=None if settings.api_base is None else substituted(settings.api_base, api_base),
        api_version=settings.api_version,
        api_key=settings.api_key,
    )


def run(call: Call, api_base: str) -> Outcome:
    apply_settings(resolved_settings(call, api_base), api_base)
    install_custom_providers(call, api_base)
    try:
        response = litellm.image_generation(**call_kwargs(call, api_base))  # pyright: ignore[reportArgumentType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L183-L561
    except APIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L853-L861
        return Outcome(failure=error_failure(error))
    finally:
        litellm.custom_provider_map = []  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L503-L506
    parsed = IMAGE.validate_python(response)
    return Outcome(result=JSON.validate_json(parsed.model_dump_json(exclude_none=True)))


class ImageGenerationCallRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    image_generation_call: Call

    def serialize(self, api_base: str) -> str:
        value = TransportValue(value=run(self.image_generation_call, api_base))
        return Envelope(image_generation_call=value).model_dump_json(by_alias=True, exclude_none=True)
