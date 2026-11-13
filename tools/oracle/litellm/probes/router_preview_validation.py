from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class RouterPreviewValidation(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "router_preview_validation is not built")
