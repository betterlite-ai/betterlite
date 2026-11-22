from collections.abc import AsyncIterator, Callable, Iterator, Mapping

from pydantic import JsonValue

from betterlite._custom_llm.base import BaseLLM
from betterlite._custom_llm.interfaces import (
    AsyncHTTPClient,
    SyncHTTPClient,
    TimeoutConfig,
    Tokenizer,
)
from betterlite._custom_llm.streaming_chunk import GenericStreamingChunk
from betterlite.types.llms.openai.core.aliases import FileTypes
from betterlite.types.llms.openai.core.messages import AllMessageValues
from betterlite.types.utils import (
    EmbeddingResponse,
    ImageResponse,
    LiteLLMLoggingBaseClass,
    ModelResponse,
)

__all__ = ["CustomLLM", "CustomLLMError"]


class CustomLLMError(Exception):
    def __init__(self, status_code: int, message: str) -> None:
        self.status_code = status_code
        self.message = message
        super().__init__(self.message)


class CustomLLM(BaseLLM):
    def completion(
        self,
        model: str,
        messages: list[AllMessageValues],
        api_base: str,
        custom_prompt_dict: dict[str, JsonValue],
        model_response: ModelResponse,
        print_verbose: Callable[[str], None],
        encoding: Tokenizer,
        api_key: str | None,
        logging_obj: LiteLLMLoggingBaseClass,
        optional_params: dict[str, JsonValue],
        acompletion: bool | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
        logger_fn: Callable[[dict[str, JsonValue]], None] | None = None,
        headers: Mapping[str, str] = {},
        timeout: float | TimeoutConfig | None = None,
        client: SyncHTTPClient | None = None,
    ) -> ModelResponse:
        raise CustomLLMError(status_code=500, message="Not implemented yet!")

    def streaming(
        self,
        model: str,
        messages: list[AllMessageValues],
        api_base: str,
        custom_prompt_dict: dict[str, JsonValue],
        model_response: ModelResponse,
        print_verbose: Callable[[str], None],
        encoding: Tokenizer,
        api_key: str | None,
        logging_obj: LiteLLMLoggingBaseClass,
        optional_params: dict[str, JsonValue],
        acompletion: bool | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
        logger_fn: Callable[[dict[str, JsonValue]], None] | None = None,
        headers: Mapping[str, str] = {},
        timeout: float | TimeoutConfig | None = None,
        client: SyncHTTPClient | None = None,
    ) -> Iterator[GenericStreamingChunk]:
        raise CustomLLMError(status_code=500, message="Not implemented yet!")

    async def acompletion(
        self,
        model: str,
        messages: list[AllMessageValues],
        api_base: str,
        custom_prompt_dict: dict[str, JsonValue],
        model_response: ModelResponse,
        print_verbose: Callable[[str], None],
        encoding: Tokenizer,
        api_key: str | None,
        logging_obj: LiteLLMLoggingBaseClass,
        optional_params: dict[str, JsonValue],
        acompletion: bool | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
        logger_fn: Callable[[dict[str, JsonValue]], None] | None = None,
        headers: Mapping[str, str] = {},
        timeout: float | TimeoutConfig | None = None,
        client: AsyncHTTPClient | None = None,
    ) -> ModelResponse:
        raise CustomLLMError(status_code=500, message="Not implemented yet!")

    async def astreaming(
        self,
        model: str,
        messages: list[AllMessageValues],
        api_base: str,
        custom_prompt_dict: dict[str, JsonValue],
        model_response: ModelResponse,
        print_verbose: Callable[[str], None],
        encoding: Tokenizer,
        api_key: str | None,
        logging_obj: LiteLLMLoggingBaseClass,
        optional_params: dict[str, JsonValue],
        acompletion: bool | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
        logger_fn: Callable[[dict[str, JsonValue]], None] | None = None,
        headers: Mapping[str, str] = {},
        timeout: float | TimeoutConfig | None = None,
        client: AsyncHTTPClient | None = None,
    ) -> AsyncIterator[GenericStreamingChunk]:
        raise CustomLLMError(status_code=500, message="Not implemented yet!")

    def image_generation(
        self,
        model: str,
        prompt: str,
        api_key: str | None,
        api_base: str | None,
        model_response: ImageResponse,
        optional_params: dict[str, JsonValue],
        logging_obj: LiteLLMLoggingBaseClass,
        timeout: float | TimeoutConfig | None = None,
        client: SyncHTTPClient | None = None,
    ) -> ImageResponse:
        raise CustomLLMError(status_code=500, message="Not implemented yet!")

    async def aimage_generation(
        self,
        model: str,
        prompt: str,
        model_response: ImageResponse,
        api_key: str | None,
        api_base: str | None,
        optional_params: dict[str, JsonValue],
        logging_obj: LiteLLMLoggingBaseClass,
        timeout: float | TimeoutConfig | None = None,
        client: AsyncHTTPClient | None = None,
    ) -> ImageResponse:
        raise CustomLLMError(status_code=500, message="Not implemented yet!")

    def embedding(
        self,
        model: str,
        input: list[str] | list[int] | list[list[int]],
        model_response: EmbeddingResponse,
        print_verbose: Callable[[str], None],
        logging_obj: LiteLLMLoggingBaseClass,
        optional_params: dict[str, JsonValue],
        api_key: str | None = None,
        api_base: str | None = None,
        timeout: float | TimeoutConfig | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
    ) -> EmbeddingResponse:
        raise CustomLLMError(status_code=500, message="Not implemented yet!")

    async def aembedding(
        self,
        model: str,
        input: list[str] | list[int] | list[list[int]],
        model_response: EmbeddingResponse,
        print_verbose: Callable[[str], None],
        logging_obj: LiteLLMLoggingBaseClass,
        optional_params: dict[str, JsonValue],
        api_key: str | None = None,
        api_base: str | None = None,
        timeout: float | TimeoutConfig | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
    ) -> EmbeddingResponse:
        raise CustomLLMError(status_code=500, message="Not implemented yet!")

    def image_edit(
        self,
        model: str,
        image: FileTypes | list[FileTypes],
        prompt: str | None,
        model_response: ImageResponse,
        api_key: str | None,
        api_base: str | None,
        optional_params: dict[str, JsonValue],
        logging_obj: LiteLLMLoggingBaseClass,
        timeout: float | TimeoutConfig | None = None,
        client: SyncHTTPClient | None = None,
    ) -> ImageResponse:
        raise CustomLLMError(status_code=500, message="Not implemented yet!")

    async def aimage_edit(
        self,
        model: str,
        image: FileTypes | list[FileTypes],
        prompt: str | None,
        model_response: ImageResponse,
        api_key: str | None,
        api_base: str | None,
        optional_params: dict[str, JsonValue],
        logging_obj: LiteLLMLoggingBaseClass,
        timeout: float | TimeoutConfig | None = None,
        client: AsyncHTTPClient | None = None,
    ) -> ImageResponse:
        raise CustomLLMError(status_code=500, message="Not implemented yet!")
