from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class QualityRouter(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "quality_router is not built")
