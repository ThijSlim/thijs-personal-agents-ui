import { useEffect, useId, useRef, useState } from "react";
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
  const base =
    NAMED[e.key] ?? (e.key.length === 1 && /[a-z]/i.test(e.key) && (e.ctrlKey || e.altKey) ? e.key.toLowerCase() : null);
  if (!base) return null;
  // Shift is part of the character for "-" and "+" typed on their own.
  const shiftIsText = (e.key === "-" || e.key === "+") && !e.ctrlKey && !e.altKey;
  const mods = [e.ctrlKey && "ctrl", e.altKey && "alt", e.shiftKey && !shiftIsText && "shift"].filter(Boolean);
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
  /** Called on Cmd+Escape so focus can leave the terminal. */
  onLeave?: () => void;
}

export function TerminalView({ paneId, label, active, ended: endedProp = false, onLeave }: Props) {
  const hintId = useId();
  const onLeaveRef = useRef(onLeave);
  onLeaveRef.current = onLeave;
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const [readEnded, setReadEnded] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const focused = useWindowFocused();
  const ended = endedProp || readEnded;
  const endedRef = useRef(ended);
  endedRef.current = ended;

  // The xterm instance lives as long as the pane is shown.
  useEffect(() => {
    const t = new Terminal({ screenReaderMode: true, scrollback: 0, cursorBlink: false, disableStdin: false });
    term.current = t;
    t.open(host.current!);
    // One send at a time, in order; consecutive queued text is merged.
    const queue: { text?: string; keys?: string[] }[] = [];
    let running = false;
    const pump = () => {
      if (running) return;
      const next = queue.shift();
      if (!next) return;
      running = true;
      sendInput(paneId, next)
        .catch((e) => setSendError(`Couldn't send input: ${message(e)}`))
        .finally(() => {
          running = false;
          pump();
        });
    };
    const send = (input: { text?: string; keys?: string[] }) => {
      if (endedRef.current) return;
      const last = queue[queue.length - 1];
      if (input.text !== undefined && last?.text !== undefined) last.text += input.text;
      else queue.push(input);
      pump();
    };
    t.attachCustomKeyEventHandler((e) => {
      if (e.isComposing || e.keyCode === 229) return true;
      if (e.metaKey && e.key === "Escape") {
        e.preventDefault();
        if (e.type === "keydown") onLeaveRef.current?.();
        return false;
      }
      const key = keyToHerdr(e);
      if (key === null) return true; // printable text arrives via onData; Cmd combos stay with the app
      e.preventDefault(); // xterm doesn't, and Tab would also move focus away
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
    setReadError(null);
    setSendError(null);
  }, [paneId]);

  useEffect(() => {
    if (term.current) term.current.options.disableStdin = ended;
  }, [ended, paneId]);

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
          setReadError(null);
          if (s.ended) setReadEnded(true);
          if (s.text === last || (s.ended && !s.text)) return;
          last = s.text;
          const lines = s.text.split(/\r?\n/);
          const width = Math.max(...lines.map((l) => l.replace(ANSI, "").length));
          term.current?.resize(Math.max(80, width), Math.max(24, lines.length));
          term.current?.write("\x1b[0m\x1b[H\x1b[2J" + lines.join("\r\n"));
        })
        .catch((e) => live && setReadError(`Couldn't read the terminal: ${message(e)}`))
        .finally(() => (inFlight = false));
    };
    tick();
    const id = setInterval(tick, SCREEN_POLL_MS);
    return () => {
      live = false;
      clearInterval(id);
    };
  }, [paneId, polling]);

  const error = sendError ?? readError;
  return (
    <div className="terminal-view">
      <p className="notice" role="status">
        {ended ? "Session ended" : ""}
      </p>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <p id={hintId} className="sr-only">
        Press Command+Escape to leave the terminal.
      </p>
      <div
        ref={host}
        className="terminal"
        role="group"
        aria-label={`Terminal for ${label}`}
        aria-describedby={hintId}
        aria-disabled={ended ? "true" : undefined}
      />
    </div>
  );
}
