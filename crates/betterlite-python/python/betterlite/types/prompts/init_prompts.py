from collections.abc import Mapping, Sequence
from datetime import datetime
from enum import StrEnum
from typing import ClassVar, Final, Literal

from pydantic import ConfigDict, JsonValue
from typing_extensions import override

from betterlite.types.llms.base import LiteLLMPydanticObjectBase
from betterlite.types.utils.base import Dumpable, InitValue


class SupportedPromptIntegrations(StrEnum):
    DOT_PROMPT = "dotprompt"
    LANGFUSE = "langfuse"
    CUSTOM = "custom"
    BITBUCKET = "bitbucket"
    GITLAB = "gitlab"
    GENERIC_PROMPT_MANAGEMENT = "generic_prompt_management"
    ARIZE_PHOENIX = "arize_phoenix"


class PromptInfo(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", protected_namespaces=())

    prompt_type: Literal["config", "db"]
    environment: str | None = "development"


class PromptLiteLLMParams(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow", protected_namespaces=())

    prompt_id: str | None = None
    prompt_integration: str
    api_base: str | None = None
    api_key: str | None = None
    provider_specific_query_params: dict[str, JsonValue] | None = None
    ignore_prompt_manager_model: bool | None = False
    # ast-grep-ignore: no-long-names
    ignore_prompt_manager_optional_params: bool | None = False
    dotprompt_content: str | None = None


def _default_prompt_type(data: dict[str, InitValue]) -> None:
    selected: Final = data["prompt_info"]
    if selected is None:
        return
    match selected:
        case Mapping():
            if selected.get("prompt_type") is None:
                data["prompt_info"] = {**selected, "prompt_type": "config"}
        case Dumpable() | Sequence() | bool() | int() | float():
            return


class PromptSpec(LiteLLMPydanticObjectBase):
    prompt_id: str
    litellm_params: PromptLiteLLMParams
    prompt_info: PromptInfo
    created_at: datetime | None = None
    updated_at: datetime | None = None
    version: int | None = None
    environment: str | None = "development"
    created_by: str | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        if "prompt_info" not in data:
            data["prompt_info"] = PromptInfo(prompt_type="config")
        else:
            _default_prompt_type(data)
        super().__init__(**data)


class PromptTemplateBase(LiteLLMPydanticObjectBase):
    litellm_prompt_id: str
    content: str
    metadata: dict[str, JsonValue] | None = None


class PromptInfoResponse(LiteLLMPydanticObjectBase):
    prompt_spec: PromptSpec
    raw_prompt_template: PromptTemplateBase | None = None
    environments: list[str] | None = None


class ListPromptsResponse(LiteLLMPydanticObjectBase):
    prompts: list[PromptSpec]
