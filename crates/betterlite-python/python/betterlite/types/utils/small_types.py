from collections.abc import Mapping
from enum import Enum
from typing import ClassVar, Literal, TypeAlias

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.grader_content import GraderContent, content_item
from betterlite.types.utils.usage import Usage
from betterlite.types.utils.values import (
    JsonValue,
    StoredValue,
    ValidationError,
    field_validator,
)


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


class FineTuningJobError(ResponseObject):
    code: str
    message: str
    param: str | None


class FineTuningJobHyperparameters(ResponseObject):
    batch_size: Literal["auto"] | int | None
    learning_rate_multiplier: Literal["auto"] | float | None
    n_epochs: Literal["auto"] | int | None


class SupervisedMethod(ResponseObject):
    hyperparameters: FineTuningJobHyperparameters | None


class DpoHyperparameters(FineTuningJobHyperparameters):
    beta: Literal["auto"] | float | None


class DpoMethod(ResponseObject):
    hyperparameters: DpoHyperparameters | None


class GraderInput(ResponseObject):
    content: GraderContent
    role: Literal["user", "assistant", "system", "developer"]
    type: Literal["message"] | None

    @field_validator("content", mode="before")
    def pick_content(self, value: StoredValue) -> StoredValue:
        return content_item(value)


class SamplingParams(ResponseObject):
    max_completions_tokens: int | None
    reasoning_effort: Literal["none", "minimal", "low", "medium", "high", "xhigh", "max"] | None
    seed: int | None
    temperature: float | None
    top_p: float | None


class StringCheckGrader(ResponseObject):
    input: str
    name: str
    operation: Literal["eq", "ne", "like", "ilike"]
    reference: str
    type: Literal["string_check"]


class TextSimilarityGrader(ResponseObject):
    evaluation_metric: Literal[
        "cosine", "fuzzy_match", "bleu", "gleu", "meteor",
        "rouge_1", "rouge_2", "rouge_3", "rouge_4", "rouge_5", "rouge_l",
    ]
    input: str
    name: str
    reference: str
    type: Literal["text_similarity"]


class PythonGrader(ResponseObject):
    name: str
    source: str
    type: Literal["python"]
    image_tag: str | None


class ScoreModelGrader(ResponseObject):
    input: list[GraderInput]
    model: str
    name: str
    type: Literal["score_model"]
    range: list[float] | None
    sampling_params: SamplingParams | None


class LabelModelGrader(ResponseObject):
    input: list[GraderInput]
    labels: list[str]
    model: str
    name: str
    passing_labels: list[str]
    type: Literal["label_model"]


MultiGraderKind: TypeAlias = (
    StringCheckGrader | TextSimilarityGrader | PythonGrader | ScoreModelGrader | LabelModelGrader
)


class MultiGrader(ResponseObject):
    calculate_output: str
    graders: MultiGraderKind
    name: str
    type: Literal["multi"]

    @field_validator("graders", mode="before")
    def pick_nested_grader(self, value: StoredValue) -> StoredValue:
        return grader_model(value, allow_label=True)


GraderKind: TypeAlias = MultiGraderKind | MultiGrader


def grader_model(value: StoredValue, *, allow_label: bool) -> StoredValue:
    match value:
        case Mapping() if "type" not in value:
            raise ValidationError("grader", ("type",), "value is required")
        case Mapping():
            selected: ResponseObject
            match value["type"]:
                case "string_check":
                    selected = StringCheckGrader(**value)
                case "text_similarity":
                    selected = TextSimilarityGrader(**value)
                case "python":
                    selected = PythonGrader(**value)
                case "score_model":
                    selected = ScoreModelGrader(**value)
                case "multi":
                    selected = MultiGrader(**value)
                case "label_model" if allow_label:
                    selected = LabelModelGrader(**value)
                case _other:
                    raise ValidationError("grader", ("type",), "unknown grader type")
            return selected
        case _other:
            return value


class ReinforcementHyperparameters(FineTuningJobHyperparameters):
    compute_multiplier: Literal["auto"] | float | None
    eval_interval: Literal["auto"] | int | None
    eval_samples: Literal["auto"] | int | None
    reasoning_effort: Literal["default", "low", "medium", "high"] | None


class ReinforcementMethod(ResponseObject):
    grader: GraderKind
    hyperparameters: ReinforcementHyperparameters | None

    @field_validator("grader", mode="before")
    def pick_grader(self, value: StoredValue) -> StoredValue:
        return grader_model(value, allow_label=False)


class FineTuningJobMethod(ResponseObject):
    type: Literal["supervised", "dpo", "reinforcement"]
    dpo: DpoMethod | None
    reinforcement: ReinforcementMethod | None
    supervised: SupervisedMethod | None


class WandbIntegration(ResponseObject):
    project: str
    entity: str | None
    name: str | None
    tags: list[str] | None


class WandbIntegrationObject(ResponseObject):
    type: Literal["wandb"]
    wandb: WandbIntegration


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
    integrations: list[WandbIntegrationObject] | None
    metadata: dict[str, str] | None
    method: FineTuningJobMethod | None
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
