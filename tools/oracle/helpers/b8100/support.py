from datetime import UTC, datetime
from typing import Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.a2a.common_utils import A2AError
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import LlmProviders, ModelResponse
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, ConfigDict, JsonValue

Body: TypeAlias = dict[str, JsonValue]

# pydantic leaves a plain dict message alone, and LiteLLM reads the prompt from exactly these fields
PROMPT: list[AllMessageValues] = [
    {"role": "user", "content": "hello there"},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
    {"role": "user", "content": "second"},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
]
EMPTY_MESSAGES: list[AllMessageValues] = []
EMPTY: Body = {}


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int | None = None
    message: str | None = None


class ChatConfig(Protocol):
    def map_openai_params(
        self,
        *,
        non_default_params: Body,
        optional_params: Body,
        model: str,
        drop_params: bool,
    ) -> Body: ...

    def validate_environment(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/a2a/chat/transformation.py#L162
        self,
        *,
        headers: dict[str, str],
        model: str,
        messages: list[AllMessageValues],
        optional_params: Body,
        litellm_params: Body,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> Body: ...

    def get_complete_url(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/a2a/chat/transformation.py#L193
        self,
        *,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Body,
        litellm_params: Body,
        stream: bool | None = None,
    ) -> str: ...

    def transform_request(
        self,
        *,
        model: str,
        messages: list[AllMessageValues],
        optional_params: Body,
        litellm_params: Body,
        headers: Body,
    ) -> Body: ...

    def transform_response(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/a2a/chat/transformation.py#L284
        self,
        *,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: Logging,
        request_data: Body,
        messages: list[AllMessageValues],
        optional_params: Body,
        litellm_params: Body,
        encoding: None,
        api_key: str | None = None,
        json_mode: bool | None = None,
    ) -> ModelResponse: ...


def config() -> ChatConfig:
    found = ProviderConfigManager.get_provider_chat_config(
        model="my-agent", provider=LlmProviders.A2A
    )
    if found is None:
        raise ValueError("no A2A chat config")
    return found


def logging_object() -> Logging:
    started = datetime(2026, 1, 1, tzinfo=UTC)
    return Logging(
        model="my-agent",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=started,
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def error_view(error: A2AError) -> JsonValue:
    failure = Failure.model_validate(error)
    return {"error": {"status": failure.status_code, "message": failure.message}}
