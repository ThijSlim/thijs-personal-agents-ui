import type { AgentStatus } from "./types";

export const STATUS_LABEL: Record<AgentStatus, string> = {
  working: "Working",
  blocked: "Blocked",
  done: "Done",
  idle: "Idle",
  unknown: "Unknown",
};

/** Agent state: a decorative dot plus a visible text label (never colour alone). */
export function StateDot({ status }: { status: AgentStatus }) {
  return (
    <span className="state">
      <span className={`state-dot state-${status}`} aria-hidden="true" />
      <span className="state-label">{STATUS_LABEL[status]}</span>
    </span>
  );
}
