from typing import ClassVar

from not_built import Unbuilt


class VectorFileReadRequest(Unbuilt):
    task: ClassVar[str] = "B1073"

    vector_file_read: None
