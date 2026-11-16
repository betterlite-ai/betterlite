import io
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
from litellm.exceptions import BadRequestError, UnsupportedParamsError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.image_edit import get_azure_ai_image_edit_config
from litellm.llms.openai.common_utils import OpenAIError
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]
Headers: TypeAlias = dict[str, str]
ImageValue: TypeAlias = bytes | io.BufferedReader | io.BytesIO | str
ImageArgument: TypeAlias = ImageValue | list[ImageValue | None] | None
# the exception classes carry Any, so the catches name them through typed aliases
UNSUPPORTED: Final[type[Exception]] = UnsupportedParamsError
BAD_REQUEST: Final[type[Exception]] = BadRequestError
OPENAI_ERROR: Final[type[Exception]] = OpenAIError


class ImageField(TypedDict):
    kind: Literal["bytes", "buffered", "reader", "unsupported"]
    text: str
    position: NotRequired[int]


class AuthStep(TypedDict):
    model: str
    api_key: NotRequired[str]
    global_api_key: NotRequired[str]
    global_api_base: NotRequired[str]
    api_base: NotRequired[str]
    environment: NotRequired[Headers]
    params: NotRequired[JsonObject]
    headers: NotRequired[Headers]


class UrlStep(TypedDict):
    model: str
    api_base: NotRequired[str]
    api_version: NotRequired[str]
    global_api_version: NotRequired[str]
    environment: NotRequired[Headers]


class TransformStep(TypedDict):
    model: str
    prompt: NotRequired[str]
    image: NotRequired[ImageField]
    images: NotRequired[list[ImageField | None]]
    options: NotRequired[JsonObject]


class ReplyStep(TypedDict):
    body: str
    status: NotRequired[int]


class Input(TypedDict):
    models: NotRequired[list[str]]
    supported: NotRequired[list[str]]
    auths: NotRequired[list[AuthStep]]
    urls: NotRequired[list[UrlStep]]
    transforms: NotRequired[list[TransformStep]]
    replies: NotRequired[list[ReplyStep]]


@runtime_checkable
class Config(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        image_edit_optional_params: JsonObject,
        model: str,
        *,
        drop_params: bool,
    ) -> JsonObject: ...

    def use_multipart_form_data(self) -> bool: ...

    def validate_environment(
        self,
        headers: Headers,
        model: str,
        api_key: str | None = None,
        litellm_params: JsonObject | None = None,
        api_base: str | None = None,
    ) -> Headers: ...

    def get_complete_url(
        self, model: str, api_base: str | None, litellm_params: JsonObject
    ) -> str: ...

    def transform_image_edit_request(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/image_edit/flux2_transformation.py#L82-L90
        self,
        model: str,
        prompt: str | None,
        image: ImageArgument,
        optional_params: JsonObject,
        litellm_params: GenericLiteLLMParams,
        headers: Headers,
    ) -> tuple[JsonObject, list[tuple[str, tuple[str, ImageValue, str]]]]: ...

    def transform_image_edit_response(
        self, model: str, raw_response: httpx.Response, logging_obj: Logging
    ) -> ImageResponse: ...


class ConfigBoundary(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: Config


class FailureRecord(BaseModel):
    status_code: int | None = None
    llm_provider: str | None = None


class LoggedDetails(BaseModel):
    input: str | None = None
    api_key: str | None = None
    original_response: str | None = None
    additional_args: JsonObject | None = None


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: LoggedDetails


JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def config_for(model: str) -> Config:
    values: Final = {"config": get_azure_ai_image_edit_config(model)}
    return ConfigBoundary.model_validate(values).config
