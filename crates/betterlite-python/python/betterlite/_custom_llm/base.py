from collections.abc import Callable
from types import TracebackType

from pydantic import JsonValue

from betterlite._custom_llm.interfaces import (
    AsyncClientSession,
    HTTPResponse,
    SyncHTTPClient,
    Tokenizer,
)
from betterlite.types.llms.openai.core.messages import AllMessageValues
from betterlite.types.utils import (
    LiteLLMLoggingBaseClass,
    ModelResponse,
    TextCompletionResponse,
)

__all__ = ["BaseLLM"]


class BaseLLM:
    _client_session: SyncHTTPClient | None = None
    _aclient_session: AsyncClientSession | None = None

    # ast-grep-ignore: no-one-line-functions
    def process_response(
        self,
        model: str,
        response: HTTPResponse,
        model_response: ModelResponse,
        stream: bool,
        logging_obj: LiteLLMLoggingBaseClass,
        optional_params: dict[str, JsonValue],
        api_key: str,
        data: dict[str, JsonValue] | str,
        messages: list[AllMessageValues],
        print_verbose: Callable[[str], None],
        encoding: Tokenizer,
    ) -> ModelResponse:
        return model_response

    # ast-grep-ignore: no-one-line-functions, no-long-names
    def process_text_completion_response(
        self,
        model: str,
        response: HTTPResponse,
        model_response: TextCompletionResponse,
        stream: bool,
        logging_obj: LiteLLMLoggingBaseClass,
        optional_params: dict[str, JsonValue],
        api_key: str,
        data: dict[str, JsonValue] | str,
        messages: list[AllMessageValues],
        print_verbose: Callable[[str], None],
        encoding: Tokenizer,
    ) -> TextCompletionResponse:
        return model_response

    def __exit__(self) -> None:
        if self._client_session is not None:
            self._client_session.close()

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_val: BaseException | None,
        exc_tb: TracebackType | None,
    ) -> None:
        if self._aclient_session is not None:
            await self._aclient_session.aclose()

    def validate_environment(self, *args: JsonValue, **kwargs: JsonValue) -> None: ...
