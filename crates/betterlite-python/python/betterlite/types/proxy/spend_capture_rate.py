from typing import Final, Literal

from pydantic import BaseModel, ConfigDict, Field

SPEND_CAPTURE_RATE_MAX_RANGE_DAYS: Final = 180

SpendCaptureProvider = Literal["openai"]


class SpendCaptureRateCheckSettings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    providers: tuple[SpendCaptureProvider, ...] = Field(("openai",), min_length=1)
    threshold: float = Field(0.9, gt=0, le=1)
    lookback_days: int = Field(7, ge=1, le=SPEND_CAPTURE_RATE_MAX_RANGE_DAYS)
    openai_project_ids: tuple[str, ...] = Field(
        (),
        description=(
            "Scope the OpenAI bill to these project ids; empty compares against the whole "
            "organization. Captured spend is never scoped, so list every project LiteLLM's OpenAI "
            "keys belong to"
        ),
    )


class CaptureRateDay(BaseModel):
    model_config = ConfigDict(frozen=True)

    date: str
    captured_spend: float
    provider_spend: float
    capture_rate: float | None


class CaptureRateReport(BaseModel):
    model_config = ConfigDict(frozen=True)

    provider: SpendCaptureProvider
    start_date: str
    end_date: str
    captured_spend: float
    provider_spend: float
    capture_rate: float | None
    threshold: float
    below_threshold: bool
    days: tuple[CaptureRateDay, ...]
