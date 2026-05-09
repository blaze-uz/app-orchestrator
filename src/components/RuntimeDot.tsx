import type { HealthStatus, ProcessStatus, ProjectStatus } from "../types/domain";
import { statusLabel } from "../lib/status";

type Status = ProcessStatus | ProjectStatus | HealthStatus;

export function RuntimeDot({ status }: { status: Status | undefined }) {
  const resolved: Status = status ?? "stopped";
  const label = statusLabel(resolved);
  return (
    <span
      className={`runtime-dot ${resolved}`}
      role="img"
      aria-label={label}
      title={label}
    />
  );
}
