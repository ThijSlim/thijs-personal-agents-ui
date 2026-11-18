import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { readScreen, sendInput } from "../../core/ipc";

/** How often the screen is re-read while the terminal is on screen. */
export const SCREEN_POLL_MS = 250;

interface KeyLike {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

const NAMED: Record<string, string> = {
  Enter: "enter",
  Escape: "esc",
  Tab: "tab",
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
  Backspace: "backspace",
  Delete: "delete",
  Home: "home",
  End: "end",
  PageUp: "pageup",
  PageDown: "pagedown",
  "-": "minus",
  "+": "plus",
};

/**
 * herdr key-combo string for a key press, or `null` when it should not be
 * sent as a key: plain printable characters (sent as text) and Cmd combos
 * (app shortcuts).
 */
export function keyToHerdr(e: KeyLike): string | null {
  if (e.metaKey) return null;
  const base = NAMED[e.key] ?? (e.key.length === 1 && /[a-z]/i.test(e.key) && (e.ctrlKey || e.altKey) ? e.key.toLowerCase() : null);
  if (!base) return null;
  const mods = [e.ctrlKey && "ctrl", e.altKey && "alt", e.shiftKey && e.key === "Tab" && "shift"].filter(Boolean);
  return [...mods, base].join("+");
}

const ANSI = /\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g;
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

function useWindowFocused(): boolean {
  const [focused, setFocused] = useState(() => document.hasFocus());
  useEffect(() => {
    const on = () => setFocused(true);
    const off = () => setFocused(false);
    window.addEventListener("focus", on);
    window.addEventListener("blur", off);
    return () => {
      window.removeEventListener("focus", on);
      window.removeEventListener("blur", off);
    };
  }, []);
  return focused;
}

interface Props {
  paneId: string;
  /** Accessible name of the agent: its title or pane id. */
  label: string;
  /** Terminal tab selected and document visible. */
  active: boolean;
  /** The session is already known to be gone. */
  ended?: boolean;
}

export function TerminalView({ paneId, label, active, ended: endedProp = false }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const [readEnded, setReadEnded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const focused = useWindowFocused();
  const ended = endedProp || readEnded;
  const endedRef = useRef(ended);
  endedRef.current = ended;

  // The xterm instance lives as long as the pane is shown.
  useEffect(() => {
    const t = new Terminal({ screenReaderMode: true, scrollback: 0, cursorBlink: false, disableStdin: false });
    term.current = t;
    t.open(host.current!);
    const send = (input: { text?: string; keys?: string[] }) => {
      if (endedRef.current) return;
      sendInput(paneId, input).catch((e) => setError(`Couldn't send input: ${message(e)}`));
    };
    t.attachCustomKeyEventHandler((e) => {
      if (e.isComposing) return true;
      const key = keyToHerdr(e);
      if (key === null) return true; // printable text arrives via onData; Cmd combos stay with the app
      if (e.type === "keydown") send({ keys: [key] });
      return false;
    });
    const data = t.onData((text) => send({ text }));
    return () => {
      data.dispose();
      t.dispose();
      term.current = null;
    };
  }, [paneId]);

  useEffect(() => {
    setReadEnded(false);
    setError(null);
  }, [paneId]);

  // Poll the screen; redraw only when it changed.
  const polling = active && focused && !ended;
  useEffect(() => {
    if (!polling) return;
    let last: string | null = null;
    let inFlight = false;
    let live = true;
    const tick = () => {
      if (inFlight) return;
      inFlight = true;
      readScreen(paneId)
        .then((s) => {
          if (!live) return;
          setError(null);
          if (s.ended) setReadEnded(true);
          if (s.text === last || (s.ended && !s.text)) return;
          last = s.text;
          const lines = s.text.split(/\r?\n/);
          const width = Math.max(...lines.map((l) => l.replace(ANSI, "").length));
          term.current?.resize(Math.max(80, width), Math.max(24, lines.length));
          term.current?.write("\x1b[H\x1b[2J" + lines.join("\r\n"));
        })
        .catch((e) => live && setError(`Couldn't read the terminal: ${message(e)}`))
        .finally(() => (inFlight = false));
    };
    const id = setInterval(tick, SCREEN_POLL_MS);
    return () => {
      live = false;
      clearInterval(id);
    };
  }, [paneId, polling]);

  return (
    <div className="terminal-view">
      {ended && <p className="notice">Session ended</p>}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <div ref={host} className="terminal" role="group" aria-label={`Terminal for ${label}`} />
    </div>
  );
}
