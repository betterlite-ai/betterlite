from typing import Protocol, cast

import httpx
from litellm.llms.langflow.chat.transformation import LangFlowConfig, LangFlowError
from litellm.types.utils import ModelResponse

from .shapes import Json, Object


class MessageView(Protocol):
    role: str
    content: str | None


class ChoiceView(Protocol):
    index: int
    finish_reason: str | None
    message: MessageView


class OptionsView(Protocol):
    model: str
    choices: list[ChoiceView]


class FailureError(Exception):
    status_code: int


class ConfigView(Protocol):
    # ast-grep-ignore: no-long-names
    def _get_openai_compatible_provider_info(
        self, api_base: str | None, api_key: str | None
    ) -> tuple[str | None, str | None]: ...

    def get_supported_openai_params(self, model: str) -> list[Json]: ...

    def get_complete_url(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langflow/chat/transformation.py#L89-L105
        self,
        *,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Object,
        litellm_params: Object,
        stream: bool | None = None,
    ) -> str: ...

    def validate_environment(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langflow/chat/transformation.py#L287-L302
        self,
        *,
        headers: dict[str, str],
        model: str,
        messages: list[Object],
        optional_params: Object,
        litellm_params: Object,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> Object: ...

    def transform_request(
        self,
        *,
        model: str,
        messages: list[Object],
        optional_params: Object,
        litellm_params: Object,
        headers: dict[str, str],
    ) -> Object: ...

    def sign_request(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langflow/chat/transformation.py#L273-L285
        self,
        *,
        headers: dict[str, str],
        optional_params: Object,
        request_data: Object,
        api_base: str,
        api_key: str | None = None,
        model: str | None = None,
        stream: bool | None = None,
        fake_stream: bool | None = None,
    ) -> tuple[Object, bytes | None]: ...

    def transform_response(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langflow/chat/transformation.py#L217-L271
        self,
        *,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: None,
        request_data: Object,
        messages: list[Object],
        optional_params: Object,
        litellm_params: Object,
        encoding: None,
        api_key: str | None = None,
        json_mode: bool | None = None,
    ) -> OptionsView: ...


FAILURE = cast("type[FailureError]", LangFlowError)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langflow/chat/transformation.py#L30-L31


def config_view() -> ConfigView:
    # LiteLLM leaves LangFlowConfig and every method on it unannotated
    return cast("ConfigView", LangFlowConfig())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langflow/chat/transformation.py#L34-L317
