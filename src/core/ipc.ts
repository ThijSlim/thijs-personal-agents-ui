import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { UiState } from "./types";

export function getState(): Promise<UiState> {
  return invoke<UiState>("get_state");
}

export function onStateChanged(cb: (state: UiState) => void): Promise<UnlistenFn> {
  return listen<UiState>("state-changed", (e) => cb(e.payload));
}

export function markSeen(paneId: string): Promise<void> {
  return invoke("mark_seen", { paneId });
}

export function moveSpace(id: string, index: number): Promise<void> {
  return invoke("move_space", { workspaceId: id, insertIndex: index });
}

export function onSelectAgent(cb: (paneId: string) => void): Promise<UnlistenFn> {
  return listen<string>("select-agent", (e) => cb(e.payload));
}

const INITIAL: UiState = { connection: "connecting", spaces: [], needsYou: [] };

/** Current UI state: an initial fetch plus a live subscription. */
export function useUiState(): UiState {
  const [state, setState] = useState<UiState>(INITIAL);
  useEffect(() => {
    let live = true;
    let gotEvent = false;
    let unlisten: UnlistenFn | undefined;
    onStateChanged((s) => {
      gotEvent = true;
      if (live) setState(s);
    }).then((u) => (live ? (unlisten = u) : u()));
    getState()
      .then((s) => {
        // An event that arrived first is newer than this snapshot.
        if (live && !gotEvent) setState(s);
      })
      .catch((e) => console.error("get_state failed", e));
    return () => {
      live = false;
      unlisten?.();
    };
  }, []);
  return state;
}
