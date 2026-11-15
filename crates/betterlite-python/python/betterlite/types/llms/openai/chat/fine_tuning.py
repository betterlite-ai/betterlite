from typing import ClassVar, Literal

from pydantic import ConfigDict

from betterlite.types.utils.base import ResponseObject


class Hyperparameters(ResponseObject):
    batch_size: str | int | None = None
    learning_rate_multiplier: str | float | None = None
    n_epochs: str | int | None = None


class FineTuningJobCreate(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    model: str
    training_file: str
    hyperparameters: Hyperparameters | None = None
    suffix: str | None = None
    validation_file: str | None = None
    integrations: list[str] | None = None
    seed: int | None = None


class LiteLLMFineTuningJobCreate(FineTuningJobCreate):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")

    custom_llm_provider: Literal["openai", "azure", "vertex_ai"] | None = None
