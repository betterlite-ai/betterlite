from typing import Literal, Protocol, TypeAlias, TypedDict, Unpack, cast

import openai
from litellm.litellm_core_utils import exception_mapping_utils as mapping

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]

_FAMILIES: dict[str, str] = {
    "ContextWindowExceededError": "context_window_exceeded",
    "BadRequestError": "bad_request",
    "AuthenticationError": "authentication",
    "Timeout": "timeout",
    "RateLimitError": "rate_limit",
    "ServiceUnavailableError": "service_unavailable",
    "APIError": "api",
    "APIConnectionError": "api_connection",
}


class _StatusCodeError(Protocol):
    status_code: int


class _ProviderCauseError(Exception):
    """Stands in for the SDK exception; aleph_alpha re-raises this directly when unmatched."""

    def __init__(self, error_str: str, *, status_code: int | None, message: str | None) -> None:
        super().__init__(error_str)
        if status_code is not None:
            self.status_code = status_code
        if message is not None:
            self.message = message


class NlpCloudInput(TypedDict):
    provider: Literal["nlp_cloud"]
    model: str
    error_str: str
    status_code: int | None
    message: str | None


class TogetherAiInput(TypedDict):
    provider: Literal["together_ai"]
    model: str
    error_str: str
    status_code: int | None
    message: str | None


class AlephAlphaInput(TypedDict):
    provider: Literal["aleph_alpha"]
    model: str
    error_str: str
    status_code: int | None
    message: str | None


class OllamaDictException(TypedDict):
    kind: Literal["dict"]
    error: str


class OllamaTextException(TypedDict):
    kind: Literal["text"]
    text: str


OllamaOriginalException = OllamaDictException | OllamaTextException


class OllamaInput(TypedDict):
    provider: Literal["ollama"]
    model: str
    original_exception: OllamaOriginalException


class VllmInput(TypedDict):
    provider: Literal["vllm"]
    model: str
    status_code: int | None
    message: str | None


Input = NlpCloudInput | TogetherAiInput | AlephAlphaInput | OllamaInput | VllmInput


class _MapperArguments(TypedDict):
    model: str
    original_exception: object
    custom_llm_provider: str
    error_str: str
    exception_type: str
    exception_provider: str
    extra_information: str


class _Mapper(Protocol):
    def __call__(self, **kwargs: Unpack[_MapperArguments]) -> None: ...


def _context(model: str, provider: str) -> dict[str, JsonValue]:
    return {
        "message": "ignored",
        "model": model,
        "provider": provider,
        "max_retries": None,
        "num_retries": None,
        "body_type": None,
        "body_param": None,
        "body_code": None,
    }


def _result_for_family(family: str, model: str, provider: str, status_code: int) -> JsonValue:
    if family == "rate_limit":
        return {
            "family": family,
            "context": _context(model, provider),
            "category": "vendor_rate_limit",
            "dimension": None,
            "headers": {},
        }
    if family == "timeout":
        return {
            "family": family,
            "context": _context(model, provider),
            "original_status": status_code,
        }
    if family == "api":
        return {"family": family, "context": _context(model, provider), "status": status_code}
    return {"family": family, **_context(model, provider)}


def _classify(error: openai.APIError, model: str, provider: str) -> JsonValue:
    family = _FAMILIES[type(error).__name__]
    status_code = cast(_StatusCodeError, error).status_code
    return _result_for_family(family, model, provider, status_code)


def _call(
    mapper: _Mapper, *, model: str, original_exception: object, provider: str, error_str: str
) -> None:
    mapper(
        model=model,
        original_exception=original_exception,
        custom_llm_provider=provider,
        error_str=error_str,
        exception_type="APIError",
        exception_provider=provider,
        extra_information="",
    )


def _run_nlp_cloud(input_value: NlpCloudInput) -> JsonValue:
    model = input_value["model"]
    cause = _ProviderCauseError(
        input_value["error_str"],
        status_code=input_value["status_code"],
        message=input_value["message"],
    )
    mapper = cast(_Mapper, mapping._map_nlp_cloud_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        _call(
            mapper,
            model=model,
            original_exception=cause,
            provider="nlp_cloud",
            error_str=input_value["error_str"],
        )
    except openai.APIError as error:
        return _classify(error, model, "nlp_cloud")
    return None


def _run_together_ai(input_value: TogetherAiInput) -> JsonValue:
    model = input_value["model"]
    cause = _ProviderCauseError(
        input_value["error_str"],
        status_code=input_value["status_code"],
        message=input_value["message"],
    )
    mapper = cast(_Mapper, mapping._map_together_ai_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        _call(
            mapper,
            model=model,
            original_exception=cause,
            provider="together_ai",
            error_str=input_value["error_str"],
        )
    except openai.APIError as error:
        return _classify(error, model, "together_ai")
    return None


def _run_aleph_alpha(input_value: AlephAlphaInput) -> JsonValue:
    model = input_value["model"]
    cause = _ProviderCauseError(
        input_value["error_str"],
        status_code=input_value["status_code"],
        message=input_value["message"],
    )
    mapper = cast(_Mapper, mapping._map_aleph_alpha_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        _call(
            mapper,
            model=model,
            original_exception=cause,
            provider="aleph_alpha",
            error_str=input_value["error_str"],
        )
    except openai.APIError as error:
        return _classify(error, model, "aleph_alpha")
    except _ProviderCauseError:
        # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/exception_mapping_utils.py#L1859-L1860
        return None
    return None


def _run_ollama(input_value: OllamaInput) -> JsonValue:
    model = input_value["model"]
    original_exception = input_value["original_exception"]
    cause: object
    if original_exception["kind"] == "dict":
        cause = {"error": original_exception["error"]}
    else:
        cause = original_exception["text"]
    mapper = cast(_Mapper, mapping._map_ollama_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        _call(mapper, model=model, original_exception=cause, provider="ollama", error_str="")
    except openai.APIError as error:
        return _classify(error, model, "ollama")
    return None


def _run_vllm(input_value: VllmInput) -> JsonValue:
    model = input_value["model"]
    cause = _ProviderCauseError(
        "vllm failure", status_code=input_value["status_code"], message=input_value["message"]
    )
    mapper = cast(_Mapper, mapping._map_vllm_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        _call(mapper, model=model, original_exception=cause, provider="vllm", error_str="")
    except openai.APIError as error:
        return _classify(error, model, "vllm")
    return None


def run(input_value: Input) -> object:
    result: JsonValue
    if input_value["provider"] == "nlp_cloud":
        result = _run_nlp_cloud(input_value)
    elif input_value["provider"] == "together_ai":
        result = _run_together_ai(input_value)
    elif input_value["provider"] == "aleph_alpha":
        result = _run_aleph_alpha(input_value)
    elif input_value["provider"] == "ollama":
        result = _run_ollama(input_value)
    else:
        result = _run_vllm(input_value)
    return {"result": result}
