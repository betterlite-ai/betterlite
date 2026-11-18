from typing import ClassVar

from not_built import Unbuilt


class FineTuningReadRequest(Unbuilt):
    task: ClassVar[str] = "B1053"

    fine_tuning_read: None
