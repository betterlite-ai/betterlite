from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class CustomDimensions(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "custom_dimensions is not built")
