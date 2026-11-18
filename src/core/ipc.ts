import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Conversation, UiState } from "./types";

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

export function readConversation(paneId: string): Promise<Conversation> {
  return invoke<Conversation>("read_conversation", { paneId });
}

/** Send `text` to the agent exactly as typed (the core rejects blank text). */
export function sendReply(paneId: string, text: string): Promise<void> {
  return invoke("send_reply", { paneId, text });
}

/**
 * Answer the permission prompt the user read (`question`) with "yes". The
 * core refuses, sending nothing, if a different prompt (or none) is showing.
 */
export function approve(paneId: string, question: string): Promise<void> {
  return invoke("approve", { paneId, question });
}

/** Answer the permission prompt the user read (`question`) with "no"; see `approve`. */
export function deny(paneId: string, question: string): Promise<void> {
  return invoke("deny", { paneId, question });
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

export interface Screen {
  text: string;
  ended: boolean;
}

/** The pane's visible screen, ANSI-styled. */
export function readScreen(paneId: string): Promise<Screen> {
  return invoke<Screen>("read_screen", { paneId });
}

/** Forward typed `text` and/or herdr key-combo strings to the pane. */
export function sendInput(paneId: string, input: { text?: string; keys?: string[] }): Promise<void> {
  return invoke("send_input", { paneId, text: input.text ?? null, keys: input.keys ?? null });
}
