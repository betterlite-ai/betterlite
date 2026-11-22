from abc import ABC, abstractmethod
from typing import Final

from pydantic import JsonValue
from typing_extensions import TypedDict

from betterlite._integrations.custom_logger_types import (
    LiteLLMLoggingObj,
    StandardCallbackDynamicParams,
)
from betterlite.types.llms.openai import AllMessageValues
from betterlite.types.prompts.init_prompts import PromptSpec


class PromptManagementClient(TypedDict):
    prompt_id: str | None
    prompt_template: list[AllMessageValues]
    prompt_template_model: str | None
    # ast-grep-ignore: no-long-names
    prompt_template_optional_params: dict[str, JsonValue] | None
    completed_messages: list[AllMessageValues] | None


# ast-grep-ignore: no-long-names
def resolve_prompt_manager_ignore_flags(
    prompt_spec: PromptSpec | None,
    ignore_prompt_manager_model: bool | None,  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L24
    ignore_prompt_manager_optional_params: bool | None,  # ast-grep-ignore: no-long-names  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L25
) -> tuple[bool, bool]:
    spec_params: Final = None if prompt_spec is None else prompt_spec.litellm_params
    return (
        bool(ignore_prompt_manager_model)
        or bool(spec_params is not None and spec_params.ignore_prompt_manager_model),
        bool(ignore_prompt_manager_optional_params)  # ast-grep-ignore: no-long-names
        or bool(spec_params is not None and spec_params.ignore_prompt_manager_optional_params),
    )


def _completed(
    client: PromptManagementClient,
    client_messages: list[AllMessageValues],
    prompt_id: str | None,
) -> PromptManagementClient:
    try:
        messages: Final = client["prompt_template"] + client_messages
    except (KeyError, TypeError) as error:
        raise ValueError(f"Error compiling prompt: {error}. Prompt id={prompt_id}") from error
    client["completed_messages"] = messages
    return client


class PromptManagementBase(ABC):
    @property
    @abstractmethod
    def integration_name(self) -> str: ...

    @abstractmethod
    def should_run_prompt_management(
        self,
        prompt_id: str | None,
        prompt_spec: PromptSpec | None,
        dynamic_callback_params: StandardCallbackDynamicParams,
    ) -> bool: ...

    @abstractmethod
    def _compile_prompt_helper(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L51
        self,
        prompt_id: str | None,
        prompt_spec: PromptSpec | None,
        prompt_variables: dict[str, JsonValue] | None,
        dynamic_callback_params: StandardCallbackDynamicParams,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
    ) -> PromptManagementClient: ...

    @abstractmethod
    async def async_compile_prompt_helper(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L63
        self,
        prompt_id: str | None,
        prompt_variables: dict[str, JsonValue] | None,
        dynamic_callback_params: StandardCallbackDynamicParams,
        prompt_spec: PromptSpec | None = None,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
    ) -> PromptManagementClient: ...

    def merge_messages(
        self,
        prompt_template: list[AllMessageValues],
        client_messages: list[AllMessageValues],
    ) -> list[AllMessageValues]:
        return prompt_template + client_messages

    def compile_prompt(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L81
        self,
        prompt_id: str,
        prompt_variables: dict[str, JsonValue] | None,
        client_messages: list[AllMessageValues],
        dynamic_callback_params: StandardCallbackDynamicParams,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
        prompt_spec: PromptSpec | None = None,
    ) -> PromptManagementClient:
        compiled: Final = self._compile_prompt_helper(
            prompt_id=prompt_id,
            prompt_spec=prompt_spec,
            prompt_variables=prompt_variables,
            dynamic_callback_params=dynamic_callback_params,
            prompt_label=prompt_label,
            prompt_version=prompt_version,
        )
        return _completed(compiled, client_messages, prompt_id)

    async def async_compile_prompt(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L108
        self,
        prompt_id: str | None,
        prompt_variables: dict[str, JsonValue] | None,
        client_messages: list[AllMessageValues],
        dynamic_callback_params: StandardCallbackDynamicParams,
        prompt_spec: PromptSpec | None = None,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
    ) -> PromptManagementClient:
        compiled: Final = await self.async_compile_prompt_helper(
            prompt_id=prompt_id,
            prompt_spec=prompt_spec,
            prompt_variables=prompt_variables,
            dynamic_callback_params=dynamic_callback_params,
            prompt_label=prompt_label,
            prompt_version=prompt_version,
        )
        return _completed(compiled, client_messages, prompt_id)

    def _get_model_from_prompt(
        self, prompt_management_client: PromptManagementClient, model: str
    ) -> str:
        template_model: Final = prompt_management_client["prompt_template_model"]
        if template_model is not None:
            return template_model
        return model.replace(f"{self.integration_name}/", "")

    def post_compile_prompt_processing(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L141
        self,
        prompt_template: PromptManagementClient,
        messages: list[AllMessageValues],
        non_default_params: dict[str, JsonValue],
        model: str,
        ignore_prompt_manager_model: bool | None = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L147
        ignore_prompt_manager_optional_params: bool | None = False  # ast-grep-ignore: no-long-names  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L148
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        completed: Final = prompt_template["completed_messages"]
        template_params: Final = prompt_template["prompt_template_optional_params"]
        skip_params: Final = ignore_prompt_manager_optional_params  # ast-grep-ignore: no-long-names
        applied: Final[dict[str, JsonValue]] = (
            {} if template_params is None or skip_params else template_params
        )
        chosen_model: Final = (
            model
            if ignore_prompt_manager_model
            else self._get_model_from_prompt(prompt_management_client=prompt_template, model=model)
        )
        return (
            chosen_model,
            messages if completed is None else completed,
            {**non_default_params, **applied},
        )

    def get_chat_completion_prompt(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L166
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
        ignore_prompt_manager_model: bool | None = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L177
        ignore_prompt_manager_optional_params: bool | None = False  # ast-grep-ignore: no-long-names  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L178
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        if prompt_id is None or not self.should_run_prompt_management(
            prompt_id=prompt_id,
            prompt_spec=prompt_spec,
            dynamic_callback_params=dynamic_callback_params,
        ):
            return model, messages, non_default_params
        # LiteLLM leaves prompt_spec out of this call, so the helper sees None on the sync path
        prompt_template: Final = self.compile_prompt(
            prompt_id=prompt_id,
            prompt_variables=prompt_variables,
            client_messages=messages,
            dynamic_callback_params=dynamic_callback_params,
            prompt_label=prompt_label,
            prompt_version=prompt_version,
        )
        flags: Final = resolve_prompt_manager_ignore_flags(
            prompt_spec,
            ignore_prompt_manager_model,
            ignore_prompt_manager_optional_params,  # ast-grep-ignore: no-long-names
        )
        return self.post_compile_prompt_processing(
            prompt_template, messages, non_default_params, model, *flags
        )

    # ast-grep-ignore: no-long-names
    async def async_get_chat_completion_prompt(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L212
        self,
        model: str,
        messages: list[AllMessageValues],
        non_default_params: dict[str, JsonValue],
        prompt_id: str | None,
        prompt_variables: dict[str, JsonValue] | None,
        dynamic_callback_params: StandardCallbackDynamicParams,
        litellm_logging_obj: LiteLLMLoggingObj,
        prompt_spec: PromptSpec | None = None,
        tools: list[dict[str, JsonValue]] | None = None,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
        ignore_prompt_manager_model: bool | None = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L225
        ignore_prompt_manager_optional_params: bool | None = False  # ast-grep-ignore: no-long-names  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prompt_management_base.py#L226
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        _ = (litellm_logging_obj, tools)
        if not self.should_run_prompt_management(
            prompt_id=prompt_id,
            prompt_spec=prompt_spec,
            dynamic_callback_params=dynamic_callback_params,
        ):
            return model, messages, non_default_params
        prompt_template: Final = await self.async_compile_prompt(
            prompt_id=prompt_id,
            prompt_variables=prompt_variables,
            client_messages=messages,
            dynamic_callback_params=dynamic_callback_params,
            prompt_spec=prompt_spec,
            prompt_label=prompt_label,
            prompt_version=prompt_version,
        )
        flags: Final = resolve_prompt_manager_ignore_flags(
            prompt_spec,
            ignore_prompt_manager_model,
            ignore_prompt_manager_optional_params,  # ast-grep-ignore: no-long-names
        )
        return self.post_compile_prompt_processing(
            prompt_template, messages, non_default_params, model, *flags
        )
