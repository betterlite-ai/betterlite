from collections.abc import Mapping
from enum import Enum
from typing import ClassVar, Literal

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.usage import Usage
from betterlite.types.utils.values import JsonValue, StoredValue


class BudgetConfig(ResponseObject):
    max_budget: float | None
    budget_duration: str | None
    tpm_limit: int | None
    rpm_limit: int | None

    @override
    def __init__(self, **data: StoredValue) -> None:
        if "time_period" in data:
            data["budget_duration"] = data.pop("time_period")
        if "budget_limit" in data:
            data["max_budget"] = data.pop("budget_limit")
        declared = {
            name: value for name, value in data.items() if name in type(self).__fields__
        }
        super().__init__(**declared)


class TranscriptionUsageDurationObject(ResponseObject):  # ast-grep-ignore: no-long-names
    type: Literal["duration"]
    seconds: float


# ast-grep-ignore: no-long-names
class TranscriptionUsageInputTokenDetailsObject(ResponseObject):
    audio_tokens: int
    text_tokens: int


class TranscriptionUsageTokensObject(ResponseObject):
    type: Literal["tokens"]
    input_tokens: int
    output_tokens: int
    total_tokens: int
    # ast-grep-ignore: no-long-names
    input_token_details: TranscriptionUsageInputTokenDetailsObject


class FineTuningJobError(ResponseObject):
    code: str
    message: str
    param: str | None


class FineTuningJobHyperparameters(ResponseObject):
    batch_size: Literal["auto"] | int | None
    learning_rate_multiplier: Literal["auto"] | float | None
    n_epochs: Literal["auto"] | int | None


class TranscriptionResponse(ResponseObject):
    text: str | None
    # ast-grep-ignore: no-long-names
    usage: TranscriptionUsageDurationObject | TranscriptionUsageTokensObject | None
    _hidden_params: dict[str, StoredValue]
    _response_headers: dict[str, StoredValue] | None
    defaults: ClassVar[dict[str, StoredValue]] = {
        "_hidden_params": {}, "_response_headers": None,
    }

    @override
    def __init__(self, text: str | None = None) -> None:
        super().__init__(text=text)


class LiteLLMFineTuningJob(ResponseObject):
    id: str
    created_at: int
    error: FineTuningJobError | None
    fine_tuned_model: str | None
    finished_at: int | None
    hyperparameters: FineTuningJobHyperparameters
    model: str
    object: Literal["fine_tuning.job"]
    organization_id: str
    result_files: list[str]
    seed: int | None
    status: Literal[
        "validating_files", "queued", "running", "succeeded", "failed", "cancelled",
    ]
    trained_tokens: int | None
    training_file: str
    validation_file: str | None
    estimated_finish: int | None
    integrations: list[JsonValue] | None
    metadata: dict[str, str] | None
    method: JsonValue
    _hidden_params: dict[str, StoredValue]
    defaults: ClassVar[dict[str, StoredValue]] = {"_hidden_params": {}}

    @override
    def __init__(self, **kwargs: StoredValue) -> None:
        if "error" in kwargs:
            error = kwargs["error"]
            match error:
                case Mapping() if all(value is None for value in error.values()):
                    kwargs["error"] = None
                case _other:
                    pass
        super().__init__(**kwargs)


# ast-grep-ignore: no-long-names
class LiteLLMRealtimeStreamLoggingObject(ResponseObject):
    results: list[dict[str, JsonValue]]
    usage: Usage
    service_tier: str | None
    _hidden_params: dict[str, StoredValue]
    defaults: ClassVar[dict[str, StoredValue]] = {"_hidden_params": {}}

    @override
    def __init__(
        self, results: StoredValue = None, usage: StoredValue = None,
        service_tier: StoredValue = None, **_params: StoredValue,
    ) -> None:
        data: dict[str, StoredValue] = {"results": results, "usage": usage}
        if service_tier is not None:
            data["service_tier"] = service_tier
        super().__init__(**data)


class LiteLLMLoggingBaseClass:
    def pre_call(
        self, input: JsonValue,  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4251
        api_key: str,
        model: str | None = None,
        additional_args: dict[str, JsonValue] | None = None,
    ) -> None:
        ...

    def post_call(
        self, original_response: JsonValue,
        input: JsonValue = None,  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4254
        api_key: str | None = None,
        additional_args: dict[str, JsonValue] | None = None,
    ) -> None:
        ...


# ast-grep-ignore: no-long-names
class DynamicPromptManagementParamLiteral(str, Enum):  # noqa: UP042  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4468
    CACHE_CONTROL_INJECTION_POINTS = "cache_control_injection_points"
    KNOWLEDGE_BASES = "knowledge_bases"
    VECTOR_STORE_IDS = "vector_store_ids"

    @classmethod
    def list_all_params(cls) -> list[str]:
        return [param.value for param in cls]
