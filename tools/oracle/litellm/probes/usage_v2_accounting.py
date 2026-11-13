from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class UsageV2Accounting(Operation):
    @override
    def run(self) -> Strict:
        raise ProbeError("unknown_operation", "usage_v2_accounting is not built")
