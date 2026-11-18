from typing import ClassVar

from not_built import Unbuilt


class ContextPreparationRequest(Unbuilt):
    task: ClassVar[str] = "B973"

    messages_context_preparation: None
