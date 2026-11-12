from collections.abc import Callable
from typing import Final, NotRequired, TypeAlias, TypedDict

import litellm.utils as litellm_utils

Capability: TypeAlias = Callable[[str, str | None], bool]


class Step(TypedDict):
    function: str
    model: str
    custom_llm_provider: NotRequired[str | None]


FLAGS: Final[dict[str, Capability]] = {
    "supports_web_search": litellm_utils.supports_web_search,
    "supports_url_context": litellm_utils.supports_url_context,
    "supports_audio_input": litellm_utils.supports_audio_input,
    "supports_pdf_input": litellm_utils.supports_pdf_input,
    "supports_audio_output": litellm_utils.supports_audio_output,
    "supports_prompt_cache_breakpoint": litellm_utils.supports_prompt_cache_breakpoint,
    "supports_thinking_cache_preservation": litellm_utils.supports_thinking_cache_preservation,
    "supports_computer_use": litellm_utils.supports_computer_use,
    "supports_vision": litellm_utils.supports_vision,
    "supports_anthropic_thinking_payload": litellm_utils.supports_anthropic_thinking_payload,
    "supports_embedding_image_input": litellm_utils.supports_embedding_image_input,
    "is_vision_explicitly_disabled": litellm_utils.is_vision_explicitly_disabled,
}


def run_step(step: Step) -> bool:
    function = step["function"]
    if function not in FLAGS:
        raise ValueError(f"unknown capability function: {function}")
    return FLAGS[function](step["model"], step.get("custom_llm_provider"))
