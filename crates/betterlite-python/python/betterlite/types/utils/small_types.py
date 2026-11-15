from collections.abc import Mapping
from enum import Enum
from typing import ClassVar, Literal, TypeAlias

from pydantic import ConfigDict, JsonValue, PrivateAttr
from typing_extensions import override

from betterlite.types.utils.base import (
    DictJsonObject,
    InitValue,
    ModelValue,
    ResponseObject,
    dump_value,
)
from betterlite.types.utils.grader_content import GraderContent, GraderRole
from betterlite.types.utils.usage import Usage


class BudgetConfig(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    max_budget: float | None = None
    budget_duration: str | None = None
    tpm_limit: int | None = None
    rpm_limit: int | None = None

    @override
    def __init__(self, **data: InitValue) -> None:
        if "time_period" in data:
            data["budget_duration"] = data.pop("time_period")
        if "budget_limit" in data:
            data["max_budget"] = data.pop("budget_limit")
        super().__init__(**data)


class FineTuningJobError(ResponseObject):
    code: str
    message: str
    param: str | None = None


class FineTuningJobHyperparameters(ResponseObject):
    batch_size: Literal["auto"] | int | None = None
    learning_rate_multiplier: Literal["auto"] | float | None = None
    n_epochs: Literal["auto"] | int | None = None


class SupervisedMethod(ResponseObject):
    hyperparameters: FineTuningJobHyperparameters | None = None


class DpoHyperparameters(FineTuningJobHyperparameters):
    beta: Literal["auto"] | float | None = None


class DpoMethod(ResponseObject):
    hyperparameters: DpoHyperparameters | None = None


class GraderInput(ResponseObject):
    content: GraderContent
    role: GraderRole
    type: Literal["message"] | None = None


ReasoningEffort: TypeAlias = Literal["none", "minimal", "low", "medium", "high", "xhigh", "max"]


class SamplingParams(ResponseObject):
    max_completions_tokens: int | None = None
    reasoning_effort: ReasoningEffort | None = None
    seed: int | None = None
    temperature: float | None = None
    top_p: float | None = None


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
    image_tag: str | None = None


class ScoreModelGrader(ResponseObject):
    input: list[GraderInput]
    model: str
    name: str
    type: Literal["score_model"]
    range: list[float] | None = None
    sampling_params: SamplingParams | None = None


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


GraderKind: TypeAlias = MultiGraderKind | MultiGrader


class ReinforcementHyperparameters(FineTuningJobHyperparameters):
    compute_multiplier: Literal["auto"] | float | None = None
    eval_interval: Literal["auto"] | int | None = None
    eval_samples: Literal["auto"] | int | None = None
    reasoning_effort: Literal["default", "low", "medium", "high"] | None = None


class ReinforcementMethod(ResponseObject):
    grader: GraderKind
    hyperparameters: ReinforcementHyperparameters | None = None


class FineTuningJobMethod(ResponseObject):
    type: Literal["supervised", "dpo", "reinforcement"]
    dpo: DpoMethod | None = None
    reinforcement: ReinforcementMethod | None = None
    supervised: SupervisedMethod | None = None


class WandbIntegration(ResponseObject):
    project: str
    entity: str | None = None
    name: str | None = None
    tags: list[str] | None = None


class WandbIntegrationObject(ResponseObject):
    type: Literal["wandb"]
    wandb: WandbIntegration


class LiteLLMFineTuningJob(ResponseObject):
    id: str
    created_at: int
    error: FineTuningJobError | None = None
    fine_tuned_model: str | None = None
    finished_at: int | None = None
    hyperparameters: FineTuningJobHyperparameters
    model: str
    object: Literal["fine_tuning.job"]
    organization_id: str
    result_files: list[str]
    seed: int | None = None
    status: Literal[
        "validating_files", "queued", "running", "succeeded", "failed", "cancelled",
    ]
    trained_tokens: int | None = None
    training_file: str
    validation_file: str | None = None
    estimated_finish: int | None = None
    integrations: list[WandbIntegrationObject] | None = None
    metadata: dict[str, str] | None = None
    method: FineTuningJobMethod | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @override
    def __init__(self, **kwargs: JsonValue) -> None:
        if "error" in kwargs:
            kwargs["error"] = error_value(kwargs["error"])
        raw_hidden = kwargs.pop("_hidden_params", None)
        hidden: dict[str, JsonValue] = {}
        if raw_hidden is not None:
            match raw_hidden:
                case Mapping():
                    hidden = dict(raw_hidden)
                case bool() | int() | float() | str() | list():
                    hidden = {}
        super().__init__(**kwargs)
        self._hidden_params = hidden


def error_value(value: JsonValue) -> JsonValue:
    if value is None:
        return None
    match value:
        case Mapping() if all(item is None for item in value.values()):
            return None
        case Mapping() | bool() | int() | float() | str() | list():
            return value


# ast-grep-ignore: no-long-names
class LiteLLMRealtimeStreamLoggingObject(DictJsonObject):
    results: list[JsonValue]
    usage: Usage
    service_tier: str | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    @override
    def __init__(
        self, results: JsonValue = None, usage: ModelValue = None,
        service_tier: JsonValue = None, **_params: InitValue,
    ) -> None:
        if service_tier is None:
            super().__init__(results=results, usage=dump_value(usage))
        else:
            super().__init__(
                results=results, usage=dump_value(usage), service_tier=service_tier,
            )


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
