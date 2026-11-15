from abc import ABC, abstractmethod
from typing import Generic, TypeVar

from pydantic import BaseModel

from betterlite.types.utils.base import ResponseObject

T = TypeVar("T", bound=BaseModel)


class GuardrailConfigModel(ResponseObject, ABC, Generic[T]):
    optional_params: T | None = None

    @staticmethod
    @abstractmethod
    def ui_friendly_name() -> str: ...
