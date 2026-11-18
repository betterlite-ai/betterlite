from typing import ClassVar

from not_built import Unbuilt


class FineTuningCreateRequest(Unbuilt):
    task: ClassVar[str] = "B1052"

    fine_tuning_create: None
