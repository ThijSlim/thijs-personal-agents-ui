import { createElement } from "react";
import type { AppModule } from "../../core/modules";
import { useUiState } from "../../core/ipc";
import { AgentPanel } from "./AgentPanel";
import { Sidebar } from "./Sidebar";

function AgentMain({ paneId }: { paneId: string }) {
  return createElement(AgentPanel, { paneId, state: useUiState() });
}

export const agentsModule: AppModule = {
  id: "agents",
  title: "Agents",
  Sidebar: ({ selection, onSelect }) => createElement(Sidebar, { state: useUiState(), selection, onSelect }),
  Main: ({ selection }) =>
    selection?.module === "agents"
      ? createElement(AgentMain, { paneId: selection.id })
      : createElement("p", { className: "muted" }, "Select an agent."),
};
