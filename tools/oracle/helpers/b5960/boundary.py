import datetime as dt
import time
from collections.abc import Callable
from typing import Final, Protocol, TypeAlias, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.prompt_templates import huggingface_template_handler
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.sagemaker.chat.transformation import SagemakerChatConfig
from litellm.llms.sagemaker.completion import transformation as completion
from litellm.llms.sagemaker.completion.transformation import SagemakerConfig
from litellm.llms.sagemaker.nova.transformation import SagemakerNovaConfig
from litellm.types.utils import ModelResponse
from pydantic import JsonValue

Object: TypeAlias = dict[str, JsonValue]

FIXED_NOW: Final = 1_700_000_000
MODEL: Final = "endpoint"
COMPONENT_STATUS: Final = 418
DROP_PARAMS: Final = False
LOG_START: Final = dt.datetime.fromtimestamp(FIXED_NOW, dt.UTC)


class TransformConfig(Protocol):
    transform_request: Callable[
        [str, list[JsonValue], Object, Object, dict[str, str]],
        Object,
    ]


class CompletionConfig(TransformConfig, Protocol):
    def map_openai_params(
        self,
        non_default_params: Object,
        optional_params: Object,
        model: str,
        *,
        drop_params: bool,
    ) -> Object: ...

    transform_response: Callable[
        [
            str,
            httpx.Response,
            ModelResponse,
            Logging,
            Object,
            list[JsonValue],
            Object,
            Object,
            None,
        ],
        ModelResponse,
    ]


class ChatConfig(TransformConfig, Protocol):
    has_custom_stream_wrapper: bool
    supports_stream_param_in_request_body: bool  # ast-grep-ignore: no-long-names

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> BaseLLMException: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    validate_environment: Callable[
        [dict[str, str], str, list[JsonValue], Object, Object],
        Object,
    ]


class TemplateAcquisition(Protocol):
    _get_tokenizer_config: Callable[[str], JsonValue]
    _get_chat_template_file: Callable[[str], JsonValue]


class FrozenClock:
    def __call__(self) -> float:
        return float(FIXED_NOW)


class WordCounter:
    def __call__(self, *, text: str, count_response_tokens: bool) -> int:
        del count_response_tokens
        return len(text.split())


class UnreachableTemplate:
    def __call__(self, hf_model_name: str) -> Object:
        del hf_model_name
        return {"status": "failure"}


COMPLETION: Final[CompletionConfig] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/completion/transformation.py#L35-L40
    "CompletionConfig", SagemakerConfig()
)
CHAT: Final[ChatConfig] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L41-L47
    "ChatConfig", SagemakerChatConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L68-L70
)
NOVA: Final[ChatConfig] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/nova/transformation.py#L17-L27
    "ChatConfig", SagemakerNovaConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L68-L70
)
LOGGING: Final = Logging(
    model=MODEL,
    messages=None,
    stream=False,
    call_type="text_completion",
    start_time=LOG_START,
    litellm_call_id="call-1",
    function_id="fn-1",
)
templates = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/huggingface_template_handler.py#L41-L89
    "TemplateAcquisition", huggingface_template_handler
)
templates._get_tokenizer_config = templates._get_chat_template_file = UnreachableTemplate()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/huggingface_template_handler.py#L41-L89
time.time = FrozenClock()
completion.token_counter = WordCounter()  # type: ignore[attr-defined, assignment]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/completion/transformation.py#L22
