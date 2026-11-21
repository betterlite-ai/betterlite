from typing_extensions import TypedDict

from betterlite._custom_llm.custom_llm import CustomLLM

__all__ = ["CustomLLMItem"]


class CustomLLMItem(TypedDict):
    provider: str
    custom_handler: CustomLLM
