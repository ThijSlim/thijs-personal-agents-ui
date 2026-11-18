export type AgentStatus = "working" | "blocked" | "done" | "idle" | "unknown";

export interface Agent {
  paneId: string;
  workspaceId: string;
  tabId: string;
  agent: string | null;
  title: string | null;
  status: AgentStatus;
  stateChangeSeq: number;
}

export interface SpaceView {
  id: string;
  label: string;
  agents: Agent[];
}

export interface NeedsYouItem {
  paneId: string;
  workspaceId: string;
  spaceLabel: string;
  title: string | null;
  status: AgentStatus;
}

export type UiState = {
  connection: "connecting" | "connected" | "notRunning";
  spaces: SpaceView[];
  needsYou: NeedsYouItem[];
};
