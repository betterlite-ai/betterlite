from typing import ClassVar

from not_built import Unbuilt


class FileWriteTransportRequest(Unbuilt):
    task: ClassVar[str] = "B2482"

    vector_file_write_transport: None
