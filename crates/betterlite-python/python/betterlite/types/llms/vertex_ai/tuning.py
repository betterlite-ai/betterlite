from typing import Literal

from typing_extensions import TypedDict


class FineTuneHyperparameters(TypedDict, total=False):
    epoch_count: int | None
    learning_rate_multiplier: float | None
    adapter_size: (
        Literal[
            "ADAPTER_SIZE_UNSPECIFIED",
            "ADAPTER_SIZE_ONE",
            "ADAPTER_SIZE_FOUR",
            "ADAPTER_SIZE_EIGHT",
            "ADAPTER_SIZE_SIXTEEN",
        ]
        | None
    )


class FineTunesupervisedTuningSpec(TypedDict, total=False):
    training_dataset_uri: str
    validation_dataset: str | None
    tuned_model_display_name: str | None
    hyperParameters: FineTuneHyperparameters | None


class FineTuneJobCreate(TypedDict, total=False):
    baseModel: str
    supervisedTuningSpec: FineTunesupervisedTuningSpec
    tunedModelDisplayName: str | None


class ResponseSupervisedTuningSpec(TypedDict, total=False):
    trainingDatasetUri: str | None
    hyperParameters: FineTuneHyperparameters | None


class ResponseTuningJob(TypedDict):
    name: str | None
    tunedModelDisplayName: str | None
    baseModel: str | None
    supervisedTuningSpec: ResponseSupervisedTuningSpec | None
    state: (
        Literal[
            "JOB_STATE_PENDING",
            "JOB_STATE_RUNNING",
            "JOB_STATE_SUCCEEDED",
            "JOB_STATE_FAILED",
            "JOB_STATE_CANCELLED",
        ]
        | None
    )
    createTime: str | None
    updateTime: str | None
