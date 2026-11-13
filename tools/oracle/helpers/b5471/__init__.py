from typing import Literal, TypeAlias, TypedDict

from fastapi.exceptions import HTTPException
from litellm.exceptions import (
    BlockedPiiEntityError,
    GuardrailRaisedException,
    ModifyResponseException,
    SensitiveDataRouteException,
)
from litellm.integrations.custom_guardrail import (
    CustomGuardrail,
    get_session_id_from_request_data,
    is_guardrail_intervention,
)
from litellm.types.guardrails import GuardrailEventHooks, Mode

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class ValidateHookInput(TypedDict):
    function: Literal["validate_hook"]
    supported: list[str]
    event_hook: str | None
    event_hooks: list[str] | None
    mode_tags: dict[str, str | list[str]] | None
    mode_default: str | list[str] | None


class InterventionInput(TypedDict):
    function: Literal["intervention"]
    kind: Literal[
        "http", "modify_response", "raised", "blocked_pii", "sensitive_route", "other"
    ]
    status: int


class FailureTextInput(TypedDict):
    function: Literal["failure_text"]
    kind: Literal["raised", "blocked_pii", "sensitive_route"]
    guardrail_name: str | None
    message: str
    wrap_default_message: bool


class ViolationInput(TypedDict):
    function: Literal["violation"]
    template: str | None
    default: str
    context: dict[str, str]


class PassthroughInput(TypedDict):
    function: Literal["passthrough"]
    guardrail_name: str | None
    message: str
    request_data: dict[str, Json]
    detection_info: dict[str, Json] | None


class AdvisoryInput(TypedDict):
    function: Literal["advisory"]
    data: dict[str, Json]
    message: str


class MaskInput(TypedDict):
    function: Literal["mask"]
    content: str
    mask: str
    start: int
    end: int


class SessionInput(TypedDict):
    function: Literal["session"]
    request_data: dict[str, Json]


Input: TypeAlias = (
    ValidateHookInput
    | InterventionInput
    | FailureTextInput
    | ViolationInput
    | PassthroughInput
    | AdvisoryInput
    | MaskInput
    | SessionInput
)


def hook_accepted(input_value: ValidateHookInput) -> bool:
    event_hook: GuardrailEventHooks | list[GuardrailEventHooks] | Mode | None = None
    try:
        if input_value["event_hook"] is not None:
            event_hook = GuardrailEventHooks(input_value["event_hook"])
        if input_value["event_hooks"] is not None:
            event_hook = [GuardrailEventHooks(name) for name in input_value["event_hooks"]]
        if input_value["mode_tags"] is not None:
            event_hook = Mode(
                tags=input_value["mode_tags"], default=input_value["mode_default"]
            )
        CustomGuardrail(
            supported_event_hooks=[
                GuardrailEventHooks(name) for name in input_value["supported"]
            ],
            event_hook=event_hook,
        )
    except ValueError:
        return False
    return True


FAILURES: dict[str, Exception] = {
    "modify_response": ModifyResponseException("blocked", "gpt-4", {}),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L1218
    "raised": GuardrailRaisedException("g", "blocked"),
    "blocked_pii": BlockedPiiEntityError("EMAIL", "g"),
    "sensitive_route": SensitiveDataRouteException("local", "session"),
    "other": ValueError("plain failure"),
}


def intervention(input_value: InterventionInput) -> bool:
    if input_value["kind"] == "http":
        return is_guardrail_intervention(HTTPException(status_code=input_value["status"]))
    return is_guardrail_intervention(FAILURES[input_value["kind"]])


def failure_text(input_value: FailureTextInput) -> str:
    name = input_value["guardrail_name"]
    if input_value["kind"] == "raised":
        return GuardrailRaisedException(
            name, input_value["message"], input_value["wrap_default_message"]
        ).message
    if input_value["kind"] == "blocked_pii":
        return BlockedPiiEntityError("EMAIL", name).message
    return SensitiveDataRouteException(
        "local", "session", name, None, input_value["message"]
    ).message


def passthrough(input_value: PassthroughInput) -> dict[str, Json]:
    guardrail = CustomGuardrail(guardrail_name=input_value["guardrail_name"])
    try:
        guardrail.raise_passthrough_exception(
            input_value["message"],
            input_value["request_data"],
            input_value["detection_info"],
        )
    except ModifyResponseException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L1218
        return {
            "message": raised.message,
            "model": raised.model,
            "guardrail_name": raised.guardrail_name,
            "detection_info": raised.detection_info,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L1218
        }
    return {}


def request_operation(input_value: AdvisoryInput | MaskInput | SessionInput) -> Json:
    if input_value["function"] == "advisory":
        data = input_value["data"]
        applied = CustomGuardrail().inject_advisory_message(data, input_value["message"])
        return {"applied": applied, "data": data}
    if input_value["function"] == "mask":
        return CustomGuardrail().mask_content_in_string(
            input_value["content"],
            input_value["mask"],
            input_value["start"],
            input_value["end"],
        )
    return get_session_id_from_request_data(input_value["request_data"])


def run(input_value: Input) -> Json:
    if input_value["function"] == "validate_hook":
        return hook_accepted(input_value)
    if input_value["function"] == "intervention":
        return intervention(input_value)
    if input_value["function"] == "failure_text":
        return failure_text(input_value)
    if input_value["function"] == "passthrough":
        return passthrough(input_value)
    if input_value["function"] == "violation":
        guardrail = CustomGuardrail(violation_message_template=input_value["template"])
        return guardrail.render_violation_message(
            input_value["default"], input_value["context"]
        )
    return request_operation(input_value)
