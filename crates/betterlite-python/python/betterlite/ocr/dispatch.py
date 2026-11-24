from collections.abc import Mapping
from typing import Final, NamedTuple, NoReturn

from pydantic import BaseModel, ConfigDict, JsonValue, PrivateAttr, TypeAdapter, ValidationError

from betterlite import exceptions
from betterlite._completion.arguments import TimeoutValues
from betterlite._completion.errors import UnsupportedCallArgument, raise_native_error
from betterlite._endpoints.payload import CallPayload, CallValue, is_dropped, to_call_payload
from betterlite._native import NativeError, NativeSearchSession

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
OCR_OBJECT: Final = "ocr"
PROVIDER_NATIVE_RESPONSE_KEY: Final = "provider_native_response"
REQ_FORMAT: Final = "req_format"
SUPPORTED_REQ_FORMATS: Final[tuple[str, ...]] = ("native", "litellm")
ASYNC_FLAG: Final = "aocr"


class OCRPageDimensions(BaseModel):
    model_config = ConfigDict(protected_namespaces=())

    dpi: int | None = None
    height: int | None = None
    width: int | None = None


class OCRPageImage(BaseModel):
    model_config = ConfigDict(extra="allow", protected_namespaces=())

    image_base64: str | None = None
    bbox: dict[str, JsonValue] | None = None


class OCRPage(BaseModel):
    model_config = ConfigDict(extra="allow", protected_namespaces=())

    index: int
    markdown: str
    images: list[OCRPageImage] | None = None
    dimensions: OCRPageDimensions | None = None


class OCRUsageInfo(BaseModel):
    model_config = ConfigDict(extra="allow", protected_namespaces=())

    pages_processed: int | None = None
    pages_processed_annotation: int | None = None
    credits: float | None = None
    doc_size_bytes: int | None = None


class OCRResponse(BaseModel):
    model_config = ConfigDict(extra="allow", protected_namespaces=())

    pages: list[OCRPage]
    model: str
    document_annotation: JsonValue = None
    usage_info: OCRUsageInfo | None = None
    content: str | None = None
    tables: list[dict[str, JsonValue]] | None = None
    keyValuePairs: list[dict[str, JsonValue]] | None = None
    object: str = OCR_OBJECT
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    def set_provider_native_response(self, native_response: dict[str, JsonValue]) -> None:
        self._hidden_params[PROVIDER_NATIVE_RESPONSE_KEY] = native_response

    def get_provider_native_response(self) -> Mapping[str, JsonValue] | None:
        native_response = self._hidden_params.get(PROVIDER_NATIVE_RESPONSE_KEY)
        if native_response is None:
            return None
        try:
            return OBJECT.validate_python(native_response)
        except ValidationError:
            return None


class OcrAnswer(BaseModel):
    response: OCRResponse
    provider_native_response: JsonValue = None


OCR_ANSWER: Final[TypeAdapter[OcrAnswer]] = TypeAdapter(OcrAnswer)


class LiteLLMOcrRequest(NamedTuple):
    model: str
    document: CallValue
    api_key: str | None
    api_base: str | None
    timeout: float | TimeoutValues | None
    custom_llm_provider: str | None
    extra_headers: dict[str, JsonValue] | None
    kwargs: dict[str, CallValue]


class OcrContext(NamedTuple):
    provider: str | None
    model: str


def _text(name: str, value: CallValue) -> str:
    match value:
        case str() as text:
            return text
        case object():
            raise TypeError(f"{name} must be a string")


def _optional_text(name: str, value: CallValue) -> str | None:
    if value is None:
        return None
    return _text(name, value)


def _optional_object(name: str, value: CallValue) -> dict[str, JsonValue] | None:
    if value is None:
        return None
    try:
        return OBJECT.validate_python(JSON.validate_python(value))
    except ValidationError as error:
        raise TypeError(f"{name} must be a mapping") from error


def _optional_timeout(value: CallValue) -> float | TimeoutValues | None:
    if value is None:
        return None
    match value:
        case int() as seconds:
            return float(seconds)
        case float() as seconds:
            return seconds
        case TimeoutValues() as timeout:
            return timeout
        case object():
            raise TypeError("timeout must be a number or an httpx.Timeout")


# _bind_request: the public keywords become the request the native call reads
def _bind_request(
    model: CallValue,
    document: CallValue,
    api_key: CallValue = None,
    api_base: CallValue = None,
    timeout: CallValue = None,
    custom_llm_provider: CallValue = None,
    extra_headers: CallValue = None,
    **kwargs: CallValue,
) -> LiteLLMOcrRequest:
    return LiteLLMOcrRequest(
        model=_text("model", model),
        document=document,
        api_key=_optional_text("api_key", api_key),
        api_base=_optional_text("api_base", api_base),
        timeout=_optional_timeout(timeout),
        custom_llm_provider=_optional_text("custom_llm_provider", custom_llm_provider),
        extra_headers=_optional_object("extra_headers", extra_headers),
        kwargs=dict(kwargs),
    )


# _public_request: Python binds the public arguments, and its TypeError names the public function
def _public_request(
    name: str, args: tuple[CallValue, ...], kwargs: Mapping[str, CallValue]
) -> LiteLLMOcrRequest:
    try:
        return _bind_request(*args, **kwargs)
    except TypeError as error:
        raise TypeError(str(error).replace("_bind_request()", f"{name}()")) from None


def _context(request: LiteLLMOcrRequest) -> OcrContext:
    prefix, separator, _rest = request.model.partition("/")
    if request.custom_llm_provider is not None:
        return OcrContext(provider=request.custom_llm_provider, model=request.model)
    return OcrContext(provider=prefix if separator else None, model=request.model)


def _raise_unsupported_format(request: LiteLLMOcrRequest, value: CallValue) -> NoReturn:
    provider = _context(request).provider
    raise exceptions.UnsupportedParamsError(
        message=f"Invalid `{REQ_FORMAT}`: {value!r}. Expected 'native' or 'litellm'.",
        model=request.model.removeprefix(f"{provider}/"),
        llm_provider=provider,
    )


# req_format fails before the provider call, as LiteLLM's route host reports it
def _check_req_format(request: LiteLLMOcrRequest) -> None:
    if REQ_FORMAT not in request.kwargs:
        return
    value = request.kwargs[REQ_FORMAT]
    match value:
        case str() as text if text in SUPPORTED_REQ_FORMATS:
            return
        case object():
            _raise_unsupported_format(request, value)


# ocr(..., aocr=True) selects LiteLLM's Python fallback, which betterlite does not have
def _check_async_flag(request: LiteLLMOcrRequest) -> None:
    if ASYNC_FLAG in request.kwargs and request.kwargs[ASYNC_FLAG] is True:
        raise UnsupportedCallArgument(ASYNC_FLAG)


# project_optional_fields: only the names the native call consumes travel to the provider
def _values(request: LiteLLMOcrRequest) -> dict[str, CallValue]:
    return {
        "model": request.model,
        "document": request.document,
        "optional_params": {
            name: value
            for name, value in request.kwargs.items()
            if not is_dropped(name, value)
        },
        "timeout_seconds": request.timeout,
        "api_key": request.api_key,
        "api_base": request.api_base,
        "custom_llm_provider": request.custom_llm_provider,
        "extra_headers": request.extra_headers,
    }


def _ocr_answer(text: str) -> OCRResponse:
    envelope = OCR_ANSWER.validate_json(text)
    response = envelope.response
    provider_native_response = envelope.provider_native_response
    if provider_native_response is not None:
        response.set_provider_native_response(OBJECT.validate_python(provider_native_response))
    return response


# The binding of both public entry points; the req_format check runs before either native call.
def _prepared(
    name: str, args: tuple[CallValue, ...], kwargs: Mapping[str, CallValue]
) -> LiteLLMOcrRequest:
    request = _public_request(name, args, kwargs)
    _check_req_format(request)
    return request


def _ocr_blocking(payload: CallPayload) -> str:
    session = NativeSearchSession("{}")
    try:
        return session.ocr(payload.text, payload.blobs)
    except NativeError as error:
        raise_native_error(error)


async def _ocr_awaiting(payload: CallPayload) -> str:
    session = NativeSearchSession("{}")
    try:
        return await session.aocr(payload.text, payload.blobs)
    except NativeError as error:
        raise_native_error(error)


def ocr(*args: CallValue, **kwargs: CallValue) -> OCRResponse:
    request = _prepared("ocr", args, kwargs)
    _check_async_flag(request)
    return _ocr_answer(_ocr_blocking(to_call_payload(_values(request))))


async def aocr(*args: CallValue, **kwargs: CallValue) -> OCRResponse:
    request = _prepared("aocr", args, kwargs)
    return _ocr_answer(await _ocr_awaiting(to_call_payload(_values(request))))
