import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { UiState } from "./types";

export function getState(): Promise<UiState> {
  return invoke<UiState>("get_state");
}

export function onStateChanged(cb: (state: UiState) => void): Promise<UnlistenFn> {
  return listen<UiState>("state-changed", (e) => cb(e.payload));
}
