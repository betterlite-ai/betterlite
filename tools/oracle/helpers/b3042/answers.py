from litellm.exceptions import RateLimitError
from pydantic import JsonValue

from .fakes import RecordingCache
from .types import JSON, RaisedFields, RateLimitResponse


def window_json(counter_key: str, window_start: str, backend: str) -> JsonValue:
    record: dict[str, JsonValue] = {
        "counter_key": counter_key,
        "window_start": window_start,
        "backend": backend,
    }
    return record


def response_json(response: RateLimitResponse) -> JsonValue:
    reserved = response.get("reservation_windows")
    windows: list[JsonValue] = []
    if reserved is not None:
        windows = [
            window_json(counter_key, window_start, backend)
            for counter_key, window_start, backend in sorted(reserved)
        ]
    rendered: dict[str, JsonValue] = {
        "overall_code": response["overall_code"],
        "statuses": JSON.validate_python(response["statuses"]),
        "reservation_windows": windows,
    }
    return rendered


def effects_json(cache: RecordingCache) -> JsonValue:
    rendered: dict[str, JsonValue] = {
        "reads": cache.reads,
        "writes": cache.writes,
        "increments": cache.increments,
        "pipelines": cache.pipelines,
    }
    return rendered


def answer(
    response: JsonValue,
    itpm_reserved: int,
    otpm_reserved: int,
    raised: JsonValue,
    cache: RecordingCache,
) -> JsonValue:
    rendered: dict[str, JsonValue] = {
        "response": response,
        "itpm_reserved": itpm_reserved,
        "otpm_reserved": otpm_reserved,
        "raised": raised,
        "effects": effects_json(cache),
    }
    return rendered


def raised_json(error: RateLimitError) -> JsonValue:
    fields = RaisedFields.model_validate(error, from_attributes=True)
    rendered: dict[str, JsonValue] = {
        "status": fields.status_code,
        "headers": JSON.validate_python(fields.headers),
        "rate_limit_type": fields.rate_limit_type,
        "category": fields.category,
    }
    return rendered
