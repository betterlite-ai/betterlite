from pydantic import JsonValue
from typing_extensions import override

from betterlite._integrations.custom_logger import CustomLogger
from betterlite._integrations.custom_logger_types import StandardCallbackDynamicParams
from betterlite._integrations.prompt_management_base import (
    PromptManagementBase,
    PromptManagementClient,
)
from betterlite.types.llms.openai import AllMessageValues
from betterlite.types.prompts.init_prompts import PromptSpec


class CustomPromptManagement(CustomLogger, PromptManagementBase):
    # LiteLLM skips CustomLogger.__init__ here, so message_logging stays unset on these instances
    @override
    def __init__(
        self,
        ignore_prompt_manager_model: bool | None = False,
        # ast-grep-ignore: no-long-names
        ignore_prompt_manager_optional_params: bool | None = False,
        **kwargs: JsonValue,
    ) -> None:
        self.ignore_prompt_manager_model = ignore_prompt_manager_model
        # ast-grep-ignore: no-long-names
        self.ignore_prompt_manager_optional_params = ignore_prompt_manager_optional_params

    @override
    def get_chat_completion_prompt(
        self,
        model: str,
        messages: list[AllMessageValues],
        non_default_params: dict[str, JsonValue],
        prompt_id: str | None,
        prompt_variables: dict[str, JsonValue] | None,
        dynamic_callback_params: StandardCallbackDynamicParams,
        prompt_spec: PromptSpec | None = None,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
        ignore_prompt_manager_model: bool | None = False,
        # ast-grep-ignore: no-long-names
        ignore_prompt_manager_optional_params: bool | None = False,
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        return model, messages, non_default_params

    @property
    @override
    def integration_name(self) -> str:
        return "custom-prompt-management"

    @override
    def should_run_prompt_management(
        self,
        prompt_id: str | None,
        prompt_spec: PromptSpec | None,
        dynamic_callback_params: StandardCallbackDynamicParams,
    ) -> bool:
        return True

    @override
    def _compile_prompt_helper(
        self,
        prompt_id: str | None,
        prompt_spec: PromptSpec | None,
        prompt_variables: dict[str, JsonValue] | None,
        dynamic_callback_params: StandardCallbackDynamicParams,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
    ) -> PromptManagementClient:
        raise NotImplementedError(
            "Custom prompt management does not support compile prompt helper"
        )

    @override
    async def async_compile_prompt_helper(
        self,
        prompt_id: str | None,
        prompt_variables: dict[str, JsonValue] | None,
        dynamic_callback_params: StandardCallbackDynamicParams,
        prompt_spec: PromptSpec | None = None,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
    ) -> PromptManagementClient:
        raise NotImplementedError(
            "Custom prompt management does not support async compile prompt helper"
        )
