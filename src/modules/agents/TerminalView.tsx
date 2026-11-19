import { useEffect, useId, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { readScreen, scrollScreen, sendInput, type Scroll } from "../../core/ipc";
import { useDocumentVisible } from "../../core/useDocumentVisible";

/**
 * How often the screen is re-read while the terminal is on screen. herdr
 * doesn't push output changes; a read is well under a millisecond.
 */
export const SCREEN_POLL_MS = 50;

/** Pixels of wheel movement that scroll the pane by one line. */
export const WHEEL_PX_PER_LINE = 16;

/** Work for herdr, done one at a time in order: input, or a scroll to an offset. */
interface Op {
  text?: string;
  keys?: string[];
  scrollTo?: number;
}

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

interface Props {
  paneId: string;
  /** Accessible name of the agent: its title or pane id. */
  label: string;
  /** The Terminal tab is selected. Polling also needs the document visible. */
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
  /** Where the pane is scrolled; reads update it, the wheel moves it. */
  const scroll = useRef<Scroll>({ offset: 0, max: 0 });
  /** Scroll ops queued or running; reads don't overwrite the offset meanwhile. */
  const scrolling = useRef(0);
  /** Re-read the screen now (set while polling). */
  const refresh = useRef(() => {});
  /** Size the terminal to the panel, but never smaller than the screen content. */
  const layout = useRef<(content?: { cols: number; rows: number }) => void>(() => {});
  const [readEnded, setReadEnded] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const visible = useDocumentVisible();
  const ended = endedProp || readEnded;
  const endedRef = useRef(ended);
  endedRef.current = ended;

  // The xterm instance lives as long as the pane is shown.
  useEffect(() => {
    const t = new Terminal({ screenReaderMode: true, scrollback: 0, cursorBlink: false, disableStdin: false });
    term.current = t;
    const fit = new FitAddon();
    t.loadAddon(fit);
    t.open(host.current!);
    let content = { cols: 80, rows: 24 };
    layout.current = (next = content) => {
      content = next;
      const panel = fit.proposeDimensions();
      const cols = Math.max(content.cols, panel?.cols ?? 0);
      const rows = Math.max(content.rows, panel?.rows ?? 0);
      if (cols !== t.cols || rows !== t.rows) t.resize(cols, rows);
    };
    layout.current();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => layout.current());
    observer?.observe(host.current!);
    // One op at a time, in order; consecutive queued text is merged, as are
    // consecutive scrolls. The screen is re-read after each.
    scroll.current = { offset: 0, max: 0 };
    scrolling.current = 0;
    const queue: Op[] = [];
    let running = false;
    const run = (op: Op) =>
      op.scrollTo === undefined
        ? sendInput(paneId, op)
            .then(() => setSendError(null))
            .catch((e) => setSendError(`Couldn't send input: ${message(e)}`))
        : scrollScreen(paneId, op.scrollTo)
            .then((s) => {
              scroll.current = s;
              setSendError(null);
            })
            .catch((e) => setSendError(`Couldn't scroll: ${message(e)}`))
            .finally(() => scrolling.current--);
    const pump = () => {
      if (running) return;
      const next = queue.shift();
      if (!next) return;
      running = true;
      run(next).finally(() => {
        running = false;
        refresh.current();
        pump();
      });
    };
    const scrollTo = (offset: number) => {
      scroll.current = { ...scroll.current, offset };
      const last = queue[queue.length - 1];
      if (last?.scrollTo !== undefined) last.scrollTo = offset;
      else {
        scrolling.current++;
        queue.push({ scrollTo: offset });
      }
    };
    const send = (input: { text?: string; keys?: string[] }) => {
      if (endedRef.current) return;
      if (scroll.current.offset > 0) scrollTo(0); // typing returns to the bottom, as in a terminal
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
    let wheelPx = 0;
    t.attachCustomWheelEventHandler((e) => {
      if (Math.abs(e.deltaX) > Math.abs(e.deltaY)) return true; // sideways scrolls the panel
      e.preventDefault();
      if (endedRef.current) return false;
      wheelPx += e.deltaMode === 0 ? e.deltaY : e.deltaY * WHEEL_PX_PER_LINE * (e.deltaMode === 2 ? t.rows : 1);
      const lines = Math.trunc(wheelPx / WHEEL_PX_PER_LINE);
      wheelPx -= lines * WHEEL_PX_PER_LINE;
      const { offset, max } = scroll.current;
      const to = Math.min(max, Math.max(0, offset - lines)); // wheel up moves away from the bottom
      if (to !== offset) {
        scrollTo(to);
        pump();
      }
      return false;
    });
    const data = t.onData((text) => send({ text }));
    return () => {
      // Don't leave herdr's pane scrolled up for whoever looks at it next.
      if (scroll.current.offset > 0 && !endedRef.current) scrollScreen(paneId, 0).catch(() => {});
      observer?.disconnect();
      layout.current = () => {};
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
  const polling = active && visible && !ended;
  useEffect(() => {
    if (!polling) return;
    let last: string | null = null;
    let inFlight = false;
    let again = false;
    let live = true;
    const tick = () => {
      if (inFlight) return void (again = true);
      inFlight = true;
      readScreen(paneId)
        .then((s) => {
          if (!live) return;
          setReadError(null);
          if (s.ended) setReadEnded(true);
          scroll.current = scrolling.current ? { ...scroll.current, max: s.scroll.max } : s.scroll;
          if (s.text === last || (s.ended && !s.text)) return;
          last = s.text;
          const lines = s.text.split(/\r?\n/);
          const width = Math.max(...lines.map((l) => l.replace(ANSI, "").length));
          layout.current({ cols: Math.max(80, width), rows: Math.max(24, lines.length) });
          term.current?.write("\x1b[0m\x1b[H\x1b[2J" + lines.join("\r\n"));
        })
        .catch((e) => live && setReadError(`Couldn't read the terminal: ${message(e)}`))
        .finally(() => {
          inFlight = false;
          if (again && live) {
            again = false;
            tick();
          }
        });
    };
    refresh.current = tick;
    tick();
    const id = setInterval(tick, SCREEN_POLL_MS);
    return () => {
      live = false;
      refresh.current = () => {};
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
        className="terminal-host"
        role="group"
        aria-label={`Terminal for ${label}`}
        aria-describedby={hintId}
        aria-disabled={ended ? "true" : undefined}
      />
    </div>
  );
}
