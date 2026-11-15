from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.llms.openai_evals import (
    CreateEvalRequest,
    CreateRunRequest,
    ListEvalsParams,
    ListRunsParams,
    UpdateEvalRequest,
)
from litellm.types.router import GenericLiteLLMParams as Params
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, JsonValue

Headers: TypeAlias = dict[str, JsonValue]
Body: TypeAlias = dict[str, JsonValue]
Operation: TypeAlias = Literal["get", "delete", "cancel"]
ReplyOperation: TypeAlias = Literal[
    "create_eval",
    "list_evals",
    "get_eval",
    "update_eval",
    "delete_eval",
    "cancel_eval",
    "create_run",
    "list_runs",
    "get_run",
    "cancel_run",
    "delete_run",
]


# LiteLLM's evals config types each hook with a bare dict, so the protocol names the shapes we read
class EvalsConfig(Protocol):
    def validate_environment(
        self, headers: dict[str, str], litellm_params: Params | None
    ) -> Headers: ...

    def get_complete_url(self, api_base: str | None, endpoint: str, eval_id: str | None) -> str: ...

    def transform_create_eval_request(
        self, create_request: CreateEvalRequest, litellm_params: Params, headers: dict[str, str]
    ) -> Body: ...

    def transform_update_eval_request(
        self,
        eval_id: str,
        update_request: UpdateEvalRequest,
        api_base: str,
        litellm_params: Params,
        headers: dict[str, str],
    ) -> tuple[str, Headers, Body]: ...

    def transform_create_run_request(
        self,
        eval_id: str,
        create_request: CreateRunRequest,
        litellm_params: Params,
        headers: dict[str, str],
    ) -> tuple[str, Body]: ...

    def transform_list_evals_request(
        self, list_params: ListEvalsParams, litellm_params: Params, headers: dict[str, str]
    ) -> tuple[str, Body]: ...

    def transform_list_runs_request(
        self,
        eval_id: str,
        list_params: ListRunsParams,
        litellm_params: Params,
        headers: dict[str, str],
    ) -> tuple[str, Body]: ...

    def transform_get_eval_request(
        self, eval_id: str, api_base: str, litellm_params: Params, headers: dict[str, str]
    ) -> tuple[str, Headers]: ...

    def transform_delete_eval_request(
        self, eval_id: str, api_base: str, litellm_params: Params, headers: dict[str, str]
    ) -> tuple[str, Headers]: ...

    def transform_cancel_eval_request(
        self, eval_id: str, api_base: str, litellm_params: Params, headers: dict[str, str]
    ) -> tuple[str, Headers, Body]: ...

    def transform_get_run_request(
        self,
        eval_id: str,
        run_id: str,
        api_base: str,
        litellm_params: Params,
        headers: dict[str, str],
    ) -> tuple[str, Headers]: ...

    def transform_cancel_run_request(
        self,
        eval_id: str,
        run_id: str,
        api_base: str,
        litellm_params: Params,
        headers: dict[str, str],
    ) -> tuple[str, Headers, Body]: ...

    def transform_delete_run_request(
        self,
        eval_id: str,
        run_id: str,
        api_base: str,
        litellm_params: Params,
        headers: dict[str, str],
    ) -> tuple[str, Headers, Body]: ...

    def transform_create_eval_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...

    def transform_list_evals_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...

    def transform_get_eval_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...

    def transform_update_eval_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...

    def transform_delete_eval_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...

    def transform_cancel_eval_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...

    def transform_create_run_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...

    def transform_list_runs_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...

    def transform_get_run_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...

    def transform_cancel_run_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...

    def transform_delete_run_response(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...


class ReplyTransform(Protocol):
    def __call__(
        self, raw_response: httpx.Response, logging_obj: Logging
    ) -> BaseModel: ...


def evals_config() -> EvalsConfig:
    found = ProviderConfigManager.get_provider_evals_api_config(provider=LlmProviders.OPENAI)
    if found is None:
        raise ValueError("the evals config is not registered")
    return found


CONFIG: Final[EvalsConfig] = evals_config()

REPLY_TRANSFORMS: Final[dict[str, ReplyTransform]] = {
    "create_eval": CONFIG.transform_create_eval_response,
    "list_evals": CONFIG.transform_list_evals_response,
    "get_eval": CONFIG.transform_get_eval_response,
    "update_eval": CONFIG.transform_update_eval_response,
    "delete_eval": CONFIG.transform_delete_eval_response,
    "cancel_eval": CONFIG.transform_cancel_eval_response,
    "create_run": CONFIG.transform_create_run_response,
    "list_runs": CONFIG.transform_list_runs_response,
    "get_run": CONFIG.transform_get_run_response,
    "cancel_run": CONFIG.transform_cancel_run_response,
    "delete_run": CONFIG.transform_delete_run_response,
}


def logging_object() -> Logging:
    return Logging(
        model="gpt-4o", messages=None, stream=False, call_type="evals",
        start_time=datetime(2026, 1, 1, tzinfo=UTC), litellm_call_id="call-1", function_id="fn-1",
    )
