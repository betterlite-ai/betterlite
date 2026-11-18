from typing import ClassVar

from not_built import Unbuilt


class VectorFileWriteRequest(Unbuilt):
    task: ClassVar[str] = "B1074"

    vector_file_write: None
