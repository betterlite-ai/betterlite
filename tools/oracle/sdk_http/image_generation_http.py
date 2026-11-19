import asyncio
from datetime import UTC, datetime
from typing import Final, Literal

import httpx
from litellm.litellm_core_utils.core_helpers import (
    get_provider_response_headers_from_hidden_params,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.tokenizer import Encoding
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.image_generation.transformation import (
    BaseImageGenerationConfig,
)
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.llms.openai import AllMessageValues
from litellm.types.llms.openai import (
    OpenAIImageGenerationOptionalParams as ImageOptions,
)
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
TEXT: TypeAdapter[str] = TypeAdapter(str)
FIELDS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
STRINGS: TypeAdapter[dict[str, str]] = TypeAdapter(dict[str, str])
MESSAGES: TypeAdapter[list[JsonValue]] = TypeAdapter(list[JsonValue])
IMAGE: TypeAdapter[ImageResponse] = TypeAdapter(ImageResponse)
# the handlers take dict[str, object] maps, so the JSON-facing maps bind to that shape
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException

HANDLER: Final = BaseLLMHTTPHandler()


class Params(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_key: str | None = None
    ssl_verify: bool | None = None


class GenerationCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    prompt: str
    mode: Mode
    api_key: str | None = None
    litellm_params: Params = Field(default_factory=Params)
    optional_params: dict[str, JsonValue] = Field(default_factory=dict)
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, JsonValue] | None = None
    fake_stream: bool = False
    litellm_metadata: dict[str, JsonValue] | None = None
    multipart: bool = False
    timeout_seconds: int


class ValidateEffect(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_key: str | None
    api_base: str | None
    headers: dict[str, JsonValue]
    model: str
    messages: list[JsonValue]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]


class UrlEffect(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_base: str | None
    api_key: str | None
    model: str
    stream: bool | None


class RequestEffect(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    prompt: str
    headers: dict[str, JsonValue]


class ResponseEffect(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    request_data: dict[str, JsonValue]
    encoding: JsonValue
    api_key: str | None
    json_mode: bool | None


class PreCallEffect(BaseModel):
    model_config = ConfigDict(extra="forbid")

    input: str
    api_key: str
    api_base: str
    headers: dict[str, JsonValue]
    complete_input_dict: dict[str, JsonValue]


class Effects(BaseModel):
    model_config = ConfigDict(extra="forbid")

    validation: ValidateEffect
    url: UrlEffect
    request: RequestEffect
    response: ResponseEffect | None
    pre_call: PreCallEffect


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
    message: str
    headers: dict[str, JsonValue]


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    result: JsonValue = None
    failure: Failure | None = None
    provider_headers: dict[str, str] | None = None
    effects: Effects


class UnrecordedEffectsError(ValueError):
    pass


class PositionalPreCallError(ValueError):
    pass


class RecordingLogging(Logging):
    def __init__(self) -> None:
        super().__init__(
            model="",
            messages="",
            stream=False,
            call_type="image_generation",
            start_time=datetime(2026, 1, 1, tzinfo=UTC),
            litellm_call_id="image-generation-http",
            function_id="image-generation-http",
        )
        self.recorded: PreCallEffect | None = None

    @override
    def pre_call(self, *args: JsonValue, **values: JsonValue) -> None:
        if args:
            raise PositionalPreCallError
        additional = FIELDS.validate_python(values["additional_args"])
        self.recorded = PreCallEffect(
            input=TEXT.validate_python(values["input"]),
            api_key=TEXT.validate_python(values["api_key"]),
            api_base=TEXT.validate_python(additional["api_base"]),
            headers=FIELDS.validate_python(additional["headers"]),
            complete_input_dict=FIELDS.validate_python(additional["complete_input_dict"]),
        )


class RecordingConfig(BaseImageGenerationConfig):
    def __init__(self, *, multipart: bool) -> None:
        self.multipart = multipart
        self.validation: ValidateEffect | None = None
        self.url: UrlEffect | None = None
        self.request: RequestEffect | None = None
        self.response: ResponseEffect | None = None

    @override
    def get_supported_openai_params(self, model: str) -> list[ImageOptions]:
        return []

    @override
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]:
        return optional_params

    @override
    def validate_environment(
        self,
        headers: dict[str, JsonValue],
        model: str,
        messages: list[AllMessageValues],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, JsonValue]:
        self.validation = ValidateEffect(
            api_key=api_key,
            api_base=api_base,
            headers=headers,
            model=model,
            messages=MESSAGES.validate_python(messages),
            optional_params=optional_params,
            litellm_params=litellm_params,
        )
        return {"x": "validated"}

    @override
    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        stream: bool | None = None,
    ) -> str:
        base = "" if api_base is None else api_base
        self.url = UrlEffect(api_base=api_base, api_key=api_key, model=model, stream=stream)
        return f"{base}/images"

    @override
    def transform_image_generation_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        prompt: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, JsonValue],
    ) -> dict[str, JsonValue]:
        self.request = RequestEffect(model=model, prompt=prompt, headers=headers)
        return {"model": model, "prompt": prompt}

    @override
    def transform_image_generation_response(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ImageResponse,
        logging_obj: Logging,
        request_data: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        encoding: Encoding | None,
        api_key: str | None = None,
        json_mode: bool | None = None,
    ) -> ImageResponse:
        self.response = ResponseEffect(
            model=model,
            request_data=request_data,
            encoding=JSON.validate_python(encoding),
            api_key=api_key,
            json_mode=json_mode,
        )
        parsed = IMAGE.validate_json(raw_response.content)
        model_response.created = parsed.created
        model_response.data = parsed.data
        model_response.usage = parsed.usage
        return model_response

    @override
    def use_multipart_form_data(self) -> bool:
        return self.multipart


# The deployment map the handler reads: the mock base, then the key and certificate setting when set
def litellm_params(call: GenerationCall, api_base: str) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {"api_base": api_base}
    if call.litellm_params.api_key is not None:
        params["api_key"] = call.litellm_params.api_key
    if call.litellm_params.ssl_verify is not None:
        params["ssl_verify"] = call.litellm_params.ssl_verify
    return params


def execute(
    call: GenerationCall,
    config: RecordingConfig,
    logging_obj: RecordingLogging,
    api_base: str,
) -> ImageResponse:
    extra_headers = OBJECTS.validate_python(call.extra_headers)
    extra_body = OBJECTS.validate_python(call.extra_body)
    metadata = OBJECTS.validate_python(call.litellm_metadata)
    if call.mode == "sync":
        return IMAGE.validate_python(
            HANDLER.image_generation_handler(
                model=call.model,
                prompt=call.prompt,
                image_generation_provider_config=config,
                image_generation_optional_request_params=call.optional_params,
                custom_llm_provider="openai",
                litellm_params=litellm_params(call, api_base),
                logging_obj=logging_obj,
                timeout=float(call.timeout_seconds),
                extra_headers=extra_headers,
                extra_body=extra_body,
                fake_stream=call.fake_stream,
                litellm_metadata=metadata,
                api_key=call.api_key,
            )
        )
    return asyncio.run(
        HANDLER.async_image_generation_handler(
            model=call.model,
            prompt=call.prompt,
            image_generation_provider_config=config,
            image_generation_optional_request_params=call.optional_params,
            custom_llm_provider="openai",
            litellm_params=litellm_params(call, api_base),
            logging_obj=logging_obj,
            timeout=float(call.timeout_seconds),
            extra_headers=extra_headers,
            extra_body=extra_body,
            fake_stream=call.fake_stream,
            litellm_metadata=metadata,
            api_key=call.api_key,
        )
    )


def effects(config: RecordingConfig, logging_obj: RecordingLogging) -> Effects:
    validation = config.validation
    url = config.url
    request = config.request
    pre_call = logging_obj.recorded
    if validation is None or url is None or request is None or pre_call is None:
        raise UnrecordedEffectsError
    return Effects(
        validation=validation,
        url=url,
        request=request,
        response=config.response,
        pre_call=pre_call,
    )


def provider_headers(response: ImageResponse) -> dict[str, str] | None:
    headers = get_provider_response_headers_from_hidden_params(response)
    return None if headers is None else STRINGS.validate_python(headers)


def run_generation(call: GenerationCall, api_base: str) -> Outcome:
    config = RecordingConfig(multipart=call.multipart)
    logging_obj = RecordingLogging()
    try:
        response = execute(call, config, logging_obj, api_base)
    except PROVIDER_ERROR as error:
        failure = Failure.model_validate(error)
        return Outcome(failure=failure, effects=effects(config, logging_obj))
    return Outcome(
        result=JSON.validate_json(response.model_dump_json(exclude_none=True)),
        provider_headers=provider_headers(response),
        effects=effects(config, logging_obj),
    )


class ImageGenerationHttpValue(BaseModel):
    value: Outcome


class ImageGenerationHttpEnvelope(BaseModel):
    image_generation_http: ImageGenerationHttpValue


class ImageGenerationHttpRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    image_generation_http: GenerationCall

    def serialize(self, api_base: str) -> str:
        value = ImageGenerationHttpValue(value=run_generation(self.image_generation_http, api_base))
        envelope = ImageGenerationHttpEnvelope(image_generation_http=value)
        return envelope.model_dump_json(by_alias=True)
