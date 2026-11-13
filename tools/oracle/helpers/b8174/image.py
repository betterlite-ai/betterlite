from collections.abc import Sequence
from typing import Protocol

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.modelscope.image_generation.transformation import (
    ModelScopeImageGenerationConfig as Images,
)
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter

from .steps import ImageBodyStep, ImageParamsStep, ImageReplyStep, Step, UrlStep
from .support import (
    Object,
    Recorder,
    UnsupportedOptionError,
    install_environment,
    object_or_empty,
)


class ImageDatum(Protocol):
    url: str | None
    b64_json: str | None
    revised_prompt: str | None


class ImageReply(Protocol):
    created: int

    @property
    def data(self) -> Sequence[ImageDatum]: ...


class ImageConfig(Protocol):
    def get_supported_openai_params(self, _model: str, /) -> Sequence[str]: ...

    def map_openai_params(
        self,
        _non_default_params: Object,
        _optional_params: Object,
        _model: str,
        /,
        *,
        drop_params: bool,
    ) -> Object: ...

    def get_complete_url(
        self,
        _api_base: str | None,
        _api_key: str | None,
        _model: str,
        _optional_params: Object,
        _litellm_params: Object,
        /,
    ) -> str: ...

    # ast-grep-ignore: no-long-names
    def transform_image_generation_request(
        self,
        _model: str,
        _prompt: str,
        _optional_params: Object,
        _litellm_params: Object,
        _headers: dict[str, str],
        /,
    ) -> Object: ...

    # ast-grep-ignore: no-long-names
    def transform_image_generation_response(
        self,
        _model: str,
        _raw_response: httpx.Response,
        _model_response: ImageResponse,
        _logging_obj: LiteLLMLoggingObj,
        _request_data: Object,
        _optional_params: Object,
        _litellm_params: Object,
        _encoding: JsonValue,
        /,
    ) -> ImageResponse: ...


class ImageFactory(Protocol):
    def __call__(self) -> ImageConfig: ...


class ImageResponseFactory(Protocol):
    def __call__(self, *, created: int | None = None) -> ImageResponse: ...


IMAGES: ImageFactory = Images
IMAGE_RESPONSE: ImageResponseFactory = ImageResponse


def image_params(step: Step) -> JsonValue:
    fields = TypeAdapter(ImageParamsStep).validate_python(step)
    params = object_or_empty(fields.get("params"))
    supported = IMAGES().get_supported_openai_params(fields["model"])
    drop_params = fields.get("drop_params")
    if drop_params is None:
        drop_params = False
    kept: Object = {}
    for name, value in params.items():
        if name in supported:
            kept[name] = value
        elif not drop_params:
            raise UnsupportedOptionError(name, fields["model"])
    optional: Object = {}
    mapped = IMAGES().map_openai_params(
        kept, optional, fields["model"], drop_params=drop_params
    )
    answer: Object = {"params": mapped}
    return TypeAdapter(Object).validate_python(answer)


def image_url(step: Step) -> JsonValue:
    fields = TypeAdapter(UrlStep).validate_python(step)
    install_environment({"MODELSCOPE_API_BASE": fields.get("env_base")})
    return IMAGES().get_complete_url(fields.get("api_base"), None, "", {}, {})


def image_body(step: Step) -> JsonValue:
    fields = TypeAdapter(ImageBodyStep).validate_python(step)
    return IMAGES().transform_image_generation_request(
        fields["model"],
        fields["prompt"],
        object_or_empty(fields.get("params")),
        {},
        {},
    )


def image_datum(image: ImageDatum) -> Object:
    datum: Object = {}
    if image.url is not None:
        datum["url"] = image.url
    if image.b64_json is not None:
        datum["b64_json"] = image.b64_json
    if image.revised_prompt is not None:
        datum["revised_prompt"] = image.revised_prompt
    return datum


def image_reply(step: Step) -> JsonValue:
    fields = TypeAdapter(ImageReplyStep).validate_python(step)
    response = httpx.Response(
        fields["status"],
        content=fields["body"].encode(),
        request=httpx.Request("POST", "https://host"),
    )
    reply = IMAGES().transform_image_generation_response(
        "",
        response,
        IMAGE_RESPONSE(created=fields.get("created")),
        Recorder(),
        {},
        {},
        {},
        None,
    )
    data = reply.data
    items: list[JsonValue] = []
    if data is not None:
        items.extend(image_datum(image) for image in data)
    answer: Object = {"created": reply.created, "data": items}
    return TypeAdapter(Object).validate_python(answer)
