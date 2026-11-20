from typing import TypedDict

from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import BaseModel, ConfigDict, Field, JsonValue

from .call import AsyncEvents, Generated, SyncEvents
from .transport import DETAILS, JSON, Run


class Hidden(TypedDict):
    model_id: str
    api_base: str
    cache_key: str
    response_cost: str
    additional_headers: dict[str, JsonValue]


class StreamState(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    hidden: Hidden = Field(alias="_hidden_params")


class HttpReply(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int
    text: str


class Views(TypedDict):
    pre_call: dict[str, JsonValue]
    httpx_response: dict[str, JsonValue] | None


def details_of(logging_obj: Logging) -> dict[str, JsonValue]:
    details: dict[str, JsonValue] = logging_obj.model_call_details
    return details


def pre_call_view(logging_obj: Logging) -> dict[str, JsonValue]:
    details = details_of(logging_obj)
    args = DETAILS.validate_python(details["additional_args"])
    return {
        "input": details["input"],
        "api_base": args["api_base"],
        "headers": args["headers"],
        "complete_input_dict": args["complete_input_dict"],
    }


def http_view(logging_obj: Logging) -> dict[str, JsonValue] | None:
    details = details_of(logging_obj)
    if "httpx_response" not in details:
        return None
    reply = HttpReply.model_validate(details["httpx_response"])
    return {"status": reply.status_code, "text": reply.text}


def views_of(logging_obj: Logging) -> Views:
    return {
        "pre_call": pre_call_view(logging_obj),
        "httpx_response": http_view(logging_obj),
    }


def answer_report(
    run: Run,
    view: Views,
    response: JsonValue,
    hidden: Hidden | None,
    events: list[JsonValue],
) -> dict[str, JsonValue]:
    return {
        "request": run.recorder.view(),
        "pre_call": view["pre_call"],
        "response": response,
        "httpx_response": view["httpx_response"],
        "hidden_params": None if hidden is None else JSON.validate_python(hidden),
        "events": events,
    }


def unary_answer(run: Run, generated: Generated) -> JsonValue:
    view = views_of(run.logging_obj)
    response = DETAILS.validate_json(generated.model_dump_json())
    return JSON.validate_python(answer_report(run, view, response, None, []))


def stream_answer(
    run: Run,
    view: Views,
    stream: SyncEvents | AsyncEvents,
    events: list[JsonValue],
) -> JsonValue:
    hidden = StreamState.model_validate(stream).hidden
    return JSON.validate_python(answer_report(run, view, None, hidden, events))
