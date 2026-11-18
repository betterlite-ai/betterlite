from typing import ClassVar

from not_built import Unbuilt


class SummaryExecutionRequest(Unbuilt):
    task: ClassVar[str] = "B2432"

    summary_execution: None
