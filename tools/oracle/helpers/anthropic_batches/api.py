from collections.abc import Coroutine
from typing import Protocol, TypedDict, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.anthropic.batches.transformation import AnthropicBatchesConfig
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.utils import LiteLLMBatch, ModelResponse
from pydantic import JsonValue

ANTHROPIC_VARS = (
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_BASE",
    "ANTHROPIC_BASE_URL",
)
AUTH_HEADERS = ("authorization", "x-api-key")
EXPLICIT_HEADERS = ("accept", "anthropic-beta", "anthropic-version", "content-type")
PARSE_PREFIX = "Failed to parse Anthropic batch response: "


class AdditionalArgs(TypedDict):
    api_base: str
    headers: dict[str, str]
    complete_input_dict: dict[str, JsonValue]


class PreCall(TypedDict):
    input: str
    api_key: str
    additional_args: AdditionalArgs


class BatchesApi(Protocol):
    def validate_environment(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/batches/transformation.py#L61-L89
        self,
        headers: dict[str, str],
        model: str,
        messages: list[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, str]: ...

    def get_complete_batch_url(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/batches/transformation.py#L91-L104
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        data: dict[str, JsonValue],
    ) -> str: ...

    def transform_create_batch_request(
        self,
        model: str,
        create_batch_data: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> bytes | str | dict[str, JsonValue]: ...

    def transform_create_batch_response(  # ast-grep-ignore: no-long-names
        self,
        model: str | None,
        raw_response: httpx.Response,
        logging_obj: Logging,
        litellm_params: dict[str, JsonValue],
    ) -> LiteLLMBatch: ...

    def transform_retrieve_batch_response(  # ast-grep-ignore: no-long-names
        self,
        model: str | None,
        raw_response: httpx.Response,
        logging_obj: Logging,
        litellm_params: dict[str, JsonValue],
    ) -> LiteLLMBatch: ...

    def get_error_class(
        self,
        error_message: str,
        status_code: int,
        headers: dict[str, str] | httpx.Headers,
    ) -> BaseLLMException: ...

    def transform_response(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/batches/transformation.py#L282-L311
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: Logging,
        request_data: dict[str, JsonValue],
        messages: list[dict[str, JsonValue]],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        encoding: None,
    ) -> ModelResponse: ...


class BatchesHandlerApi(Protocol):
    def retrieve_batch(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/batches/handler.py#L124-L163
        self,
        *,
        _is_async: bool,
        batch_id: str,
        api_base: str | None,
        api_key: str | None,
        timeout: float | httpx.Timeout,
        max_retries: int | None,
        logging_obj: Logging | None = None,
    ) -> LiteLLMBatch: ...

    def aretrieve_batch(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/batches/handler.py#L37-L119
        self,
        batch_id: str,
        api_base: str | None,
        api_key: str | None,
        timeout: float | httpx.Timeout,
        max_retries: int | None,
        logging_obj: Logging | None = None,
    ) -> Coroutine[None, None, LiteLLMBatch]: ...


CONFIG = cast(BatchesApi, AnthropicBatchesConfig())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/batches/transformation.py#L49-L54
