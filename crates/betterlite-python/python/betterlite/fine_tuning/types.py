from pydantic import JsonValue

from betterlite.types.utils import LiteLLMFineTuningJob
from betterlite.types.utils.base import ResponseObject, validated_value
from betterlite.types.utils.small_types import error_value


class FineTuningJobPage(ResponseObject):
    data: list[LiteLLMFineTuningJob]
    has_more: bool = False
    object: str = "list"
    first_id: str | None = None
    last_id: str | None = None


# The page carries the provider's job dicts; LiteLLM's job maps an all-null error to None first.
def job_fields(value: JsonValue) -> JsonValue:
    match value:
        case dict() as fields:
            return {**fields, "error": error_value(fields.get("error"))}
        case None | bool() | int() | float() | str() | list():
            return value


def page_value(value: JsonValue) -> FineTuningJobPage:
    match value:
        case dict() as page:
            loaded = dict(page)
            data = page.get("data")
            match data:
                case list() as jobs:
                    loaded["data"] = [job_fields(job) for job in jobs]
                case None | bool() | int() | float() | str() | dict():
                    pass
            return validated_value(loaded, FineTuningJobPage)
        case None | bool() | int() | float() | str() | list():
            raise TypeError("a fine-tuning job page must be an object")
