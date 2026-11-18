import asyncio
from datetime import UTC, date, datetime, timedelta
from typing import Final, Literal, Protocol, TypedDict

import litellm
from litellm.integrations.SlackAlerting.slack_alerting import SlackAlerting
from litellm.proxy._types import WebhookEvent
from litellm.router import Router
from litellm.types.integrations.slack_alerting import AlertType
from pydantic import JsonValue
from typing_extensions import override

DEPRECATED_OFFSET_DAYS: Final[int] = -5
IMMINENT_OFFSET_DAYS: Final[int] = 10
UPCOMING_OFFSET_DAYS: Final[int] = 100


class ModelAddedInput(TypedDict):
    function: Literal["model_added"]
    model_name: str
    litellm_model_name: str
    base_model: str | None
    model_info: dict[str, dict[str, JsonValue]]
    proxy_base_url: str | None


class DeprecationInput(TypedDict):
    function: Literal["deprecation_alert"]
    deprecated: int
    imminent: int
    upcoming: int


Input = ModelAddedInput | DeprecationInput


class Answer(TypedDict, total=False):
    message: str
    level: str | None
    metadata: dict[str, int] | None


class CapturedAlert(TypedDict):
    message: str
    level: str
    alert_type: str
    metadata: dict[str, int]
    user_info: WebhookEvent | None
    request_model: str | None
    api_base: str | None
    kwargs: dict[str, JsonValue]


class DeploymentRow(TypedDict):
    model_name: str
    litellm_params: dict[str, str]
    model_info: dict[str, str]


class RouterStorage(Protocol):
    model_list: list[DeploymentRow]


class PassedModelInfo:
    def __init__(self, base_model: str | None) -> None:
        self.base_model = base_model


class ModelAddedCaller(Protocol):
    async def model_added_alert(
        self,
        model_name: str,
        litellm_model_name: str,
        passed_model_info: PassedModelInfo,
    ) -> None: ...


class AlertCapture(SlackAlerting):
    sent: list[CapturedAlert]

    @override
    async def send_alert(
        self,
        message: str,
        level: Literal["Low", "Medium", "High"],
        alert_type: AlertType,
        alerting_metadata: dict[str, int],
        user_info: WebhookEvent | None = None,
        request_model: str | None = None,
        api_base: str | None = None,
        **kwargs: JsonValue,
    ) -> None:
        self.sent.append(
            {
                "message": message,
                "level": level,
                "alert_type": alert_type.value,
                "metadata": dict(alerting_metadata),
                "user_info": user_info,
                "request_model": request_model,
                "api_base": api_base,
                "kwargs": dict(kwargs),
            }
        )


def deprecation_row(name: str, deprecation_date: date) -> DeploymentRow:
    return {
        "model_name": name,
        "litellm_params": {"model": f"openai/{name}"},
        "model_info": {"deprecation_date": deprecation_date.isoformat()},
    }


def bucket_rows(prefix: str, count: int, offset_days: int, today: date) -> list[DeploymentRow]:
    return [
        deprecation_row(f"{prefix}-{index}", today + timedelta(days=offset_days))
        for index in range(count)
    ]


def deprecation_rows(input_value: DeprecationInput) -> list[DeploymentRow]:
    today = datetime.now(UTC).date()
    return [
        *bucket_rows("deprecated", input_value["deprecated"], DEPRECATED_OFFSET_DAYS, today),
        *bucket_rows("imminent", input_value["imminent"], IMMINENT_OFFSET_DAYS, today),
        *bucket_rows("upcoming", input_value["upcoming"], UPCOMING_OFFSET_DAYS, today),
    ]


def model_added(input_value: ModelAddedInput) -> Answer:
    litellm.model_cost = input_value["model_info"]
    capture = AlertCapture()
    capture.sent = []
    caller: ModelAddedCaller = capture
    asyncio.run(
        caller.model_added_alert(
            input_value["model_name"],
            input_value["litellm_model_name"],
            PassedModelInfo(input_value["base_model"]),
        )
    )
    return {"message": capture.sent[0]["message"]}


def deprecation_alert(input_value: DeprecationInput) -> Answer:
    router = Router()
    storage: RouterStorage = router
    storage.model_list = deprecation_rows(input_value)
    capture = AlertCapture()
    capture.sent = []
    sent = asyncio.run(capture.send_model_deprecation_alert(llm_router=router))
    if not sent:
        return {"level": None, "metadata": None}
    alert = capture.sent[0]
    return {"level": alert["level"], "metadata": alert["metadata"]}


def run(input_value: Input) -> Answer:
    if input_value["function"] == "model_added":
        return model_added(input_value)
    return deprecation_alert(input_value)
