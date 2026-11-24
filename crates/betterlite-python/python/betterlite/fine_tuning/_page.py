from pydantic import JsonValue, field_validator

from betterlite.types.utils import LiteLLMFineTuningJob
from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.small_types import error_value


# LiteLLM's job maps an all-null error to None, and every job a page carries does the same first.
class NormalizedJob(LiteLLMFineTuningJob):
    @field_validator("error", mode="before")
    @classmethod
    def normalize_error(cls, value: JsonValue) -> JsonValue:
        return error_value(value)


class FineTuningJobPage(ResponseObject):
    data: list[NormalizedJob]
    has_more: bool = False
    object: str = "list"
    first_id: str | None = None
    last_id: str | None = None
