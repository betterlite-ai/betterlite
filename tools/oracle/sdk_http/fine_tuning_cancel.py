from typing import ClassVar

from not_built import Unbuilt


class FineTuningCancelRequest(Unbuilt):
    task: ClassVar[str] = "B1141"

    fine_tuning_cancel: None
