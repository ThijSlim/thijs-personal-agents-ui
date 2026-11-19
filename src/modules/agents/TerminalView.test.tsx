import { act, fireEvent, render, screen } from "@testing-library/react";
import { axe } from "vitest-axe";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readScreen, scrollScreen, sendInput, sendText } from "../../core/ipc";
import { MOUSE_OFF, MOUSE_ON, SCREEN_POLL_MS, TerminalView, WHEEL_PX_PER_LINE, keyToHerdr } from "./TerminalView";

vi.mock("../../core/ipc", () => ({
  readScreen: vi.fn(),
  sendInput: vi.fn().mockResolvedValue(undefined),
  sendText: vi.fn().mockResolvedValue(undefined),
  scrollScreen: vi.fn(),
}));

const h = vi.hoisted(() => ({ terms: [] as any[] }));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    writes: string[] = [];
    disposed = false;
    onData = (cb: (d: string) => void) => ((this as any).data = cb, { dispose() {} });
    keyHandler?: (e: KeyboardEvent) => boolean;
    attachCustomKeyEventHandler = (f: (e: KeyboardEvent) => boolean) => void (this.keyHandler = f);
    wheelHandler?: (e: WheelEvent) => boolean;
    attachCustomWheelEventHandler = (f: (e: WheelEvent) => boolean) => void (this.wheelHandler = f);
    constructor(public opts: any) {
      h.terms.push(this);
    }
    options: any = {};
    cols = 80;
    rows = 24;
    loadAddon = vi.fn();
    open = vi.fn();
    resize = vi.fn((c: number, r: number) => void ((this.cols = c), (this.rows = r)));
    write = (s: string) => void this.writes.push(s);
    focus = vi.fn();
    dispose = () => void (this.disposed = true);
  },
}));
vi.mock("@xterm/xterm/css/xterm.css", () => ({}));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { proposeDimensions = () => undefined; } }));

const key = (key: string, o: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {}) =>
  ({ key, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...o });

describe("keyToHerdr", () => {
  it.each([
    [key("Enter"), "enter"],
    [key("Escape"), "esc"],
    [key("Tab"), "tab"],
    [key("Tab", { shiftKey: true }), "shift+tab"],
    [key("ArrowUp"), "up"],
    [key("ArrowDown"), "down"],
    [key("ArrowLeft"), "left"],
    [key("ArrowRight"), "right"],
    [key("Backspace"), "backspace"],
    [key("c", { ctrlKey: true }), "ctrl+c"],
    [key("-"), "minus"],
    [key("+"), "plus"],
    [key("Enter", { shiftKey: true }), "shift+enter"],
    [key("ArrowUp", { shiftKey: true }), "shift+up"],
    [key("c", { ctrlKey: true, shiftKey: true }), "ctrl+shift+c"],
    [key("+", { shiftKey: true }), "plus"],
    [key("A", { shiftKey: true }), null],
    [key("a"), null],
    [key(" "), null],
    [key("c", { metaKey: true }), null],
    [key("Enter", { metaKey: true }), null],
    [key("w", { metaKey: true }), null],
  ])("maps %o to %s", (e, want) => expect(keyToHerdr(e)).toBe(want));
});

const setVisibility = (v: DocumentVisibilityState) => {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => v });
  document.dispatchEvent(new Event("visibilitychange"));
};
const flush = async (ms = 0) => act(async () => void (await vi.advanceTimersByTimeAsync(ms)));
const term = () => h.terms[h.terms.length - 1];
const shown = (text: string, ended = false, offset = 0, max = 100) => ({ text, ended, scroll: { offset, max } });
const wheel = (deltaY: number, o: Partial<WheelEvent> = {}) =>
  ({ type: "wheel", deltaY, deltaX: 0, deltaMode: 0, preventDefault: vi.fn(), ...o }) as unknown as WheelEvent;

describe("TerminalView", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    h.terms.length = 0;
    vi.mocked(readScreen).mockResolvedValue(shown("hello", false));
    setVisibility("visible");
  });
  afterEach(() => {
    vi.useRealTimers();
    setVisibility("visible");
  });

  const view = (over: Partial<Parameters<typeof TerminalView>[0]> = {}) =>
    render(<TerminalView paneId="w1:p1" label="fix bug" active {...over} />);

  it("polls only while visible", async () => {
    const { rerender } = view({ active: false });
    await flush(1000);
    expect(readScreen).not.toHaveBeenCalled();
    rerender(<TerminalView paneId="w1:p1" label="fix bug" active />);
    await flush(0);
    expect(readScreen).toHaveBeenCalledTimes(1); // immediately
    await flush(2 * SCREEN_POLL_MS);
    expect(readScreen).toHaveBeenCalledTimes(3);
    rerender(<TerminalView paneId="w1:p1" label="fix bug" active={false} />);
    await flush(1000);
    expect(readScreen).toHaveBeenCalledTimes(3);
  });

  it("keeps polling while the window is visible but not focused", async () => {
    view();
    await flush(0);
    expect(readScreen).toHaveBeenCalledTimes(1);
    act(() => void window.dispatchEvent(new Event("blur")));
    await flush(2 * SCREEN_POLL_MS);
    expect(readScreen).toHaveBeenCalledTimes(3);
  });

  it("pauses while the document is hidden and resumes when visible", async () => {
    view();
    await flush(0);
    expect(readScreen).toHaveBeenCalledTimes(1);
    act(() => setVisibility("hidden"));
    await flush(1000);
    expect(readScreen).toHaveBeenCalledTimes(1);
    act(() => setVisibility("visible"));
    await flush(0);
    expect(readScreen).toHaveBeenCalledTimes(2);
  });

  it("does not poll when the document starts hidden", async () => {
    setVisibility("hidden");
    view();
    await flush(1000);
    expect(readScreen).not.toHaveBeenCalled();
  });

  it("redraws only when the text changed", async () => {
    view();
    await flush(0);
    expect(term().writes).toHaveLength(1);
    expect(term().writes[0]).toBe("\x1b[0m\x1b[H\x1b[2Jhello");
    await flush(2 * SCREEN_POLL_MS);
    expect(readScreen).toHaveBeenCalledTimes(3);
    expect(term().writes).toHaveLength(1);
    vi.mocked(readScreen).mockResolvedValue(shown("a\nb", false));
    await flush(SCREEN_POLL_MS);
    expect(term().writes).toHaveLength(2);
    expect(term().writes[1]).toBe("\x1b[0m\x1b[H\x1b[2Ja\r\nb");
  });

  it("cleans up interval and terminal on unmount", async () => {
    const { unmount } = view();
    await flush(0);
    unmount();
    expect(term().disposed).toBe(true);
    await flush(1000);
    expect(readScreen).toHaveBeenCalledTimes(1);
  });

  it("forwards special keys as keys and typed text raw", async () => {
    view();
    const t = term();
    const ev = (k: string, o = {}) => ({ ...key(k, o), preventDefault: vi.fn(), type: "keydown" }) as any;
    expect(t.keyHandler(ev("Enter"))).toBe(false);
    expect(sendInput).toHaveBeenCalledWith("w1:p1", { keys: ["enter"] });
    expect(t.keyHandler(ev("c", { metaKey: true }))).toBe(true);
    expect(t.keyHandler(ev("a"))).toBe(true);
    t.data("ab");
    await flush(0);
    expect(sendText).toHaveBeenCalledWith("w1:p1", "ab");
    expect(sendInput).toHaveBeenCalledTimes(1);
  });

  it("ended disables input and says so", async () => {
    vi.mocked(readScreen).mockResolvedValue(shown("bye", true));
    view();
    await flush(0);
    expect(screen.getByRole("status")).toHaveTextContent("Session ended");
    expect(screen.getByRole("group")).toHaveAttribute("aria-disabled", "true");
    const t = term();
    expect(t.options.disableStdin).toBe(true);
    t.data("x");
    t.keyHandler({ ...key("Enter"), type: "keydown", preventDefault: vi.fn() });
    expect(sendInput).not.toHaveBeenCalled();
    expect(sendText).not.toHaveBeenCalled();
    await flush(1000);
    expect(readScreen).toHaveBeenCalledTimes(1);
  });

  it("prevents default for every mapped key", () => {
    view();
    for (const [k, o] of [["Tab", {}], ["Tab", { shiftKey: true }], ["Enter", {}], ["ArrowUp", {}], ["ArrowLeft", {}]] as const) {
      const e = { ...key(k, o), type: "keydown", preventDefault: vi.fn() } as any;
      expect(term().keyHandler(e)).toBe(false);
      expect(e.preventDefault).toHaveBeenCalledTimes(1);
    }
    const printable = { ...key("a"), type: "keydown", preventDefault: vi.fn() } as any;
    term().keyHandler(printable);
    expect(printable.preventDefault).not.toHaveBeenCalled();
  });

  it("Cmd+Escape leaves the terminal and sends nothing", () => {
    const onLeave = vi.fn();
    view({ onLeave });
    const e = { ...key("Escape", { metaKey: true }), type: "keydown", preventDefault: vi.fn() } as any;
    expect(term().keyHandler(e)).toBe(false);
    expect(onLeave).toHaveBeenCalledTimes(1);
    expect(sendInput).not.toHaveBeenCalled();
    const hint = screen.getByRole("group").getAttribute("aria-describedby")!;
    expect(document.getElementById(hint)).toHaveTextContent("Press Command+Escape to leave the terminal.");
  });

  it("ignores IME keydowns", () => {
    view();
    const e = { ...key("Enter"), keyCode: 229, type: "keydown", preventDefault: vi.fn() } as any;
    expect(term().keyHandler(e)).toBe(true);
    expect(sendInput).not.toHaveBeenCalled();
  });

  it("sends in order, one at a time", async () => {
    let release!: () => void;
    const order: string[] = [];
    let inFlight = 0;
    const record = async (sent: object) => {
      inFlight++;
      expect(inFlight).toBe(1);
      order.push(JSON.stringify(sent));
      if (order.length === 1) await new Promise<void>((r) => (release = r));
      inFlight--;
    };
    vi.mocked(sendInput).mockImplementation((_p, i) => record(i));
    vi.mocked(sendText).mockImplementation((_p, text) => record({ text }));
    view();
    const t = term();
    t.data("l");
    t.data("s");
    t.data("!");
    t.keyHandler({ ...key("Enter"), type: "keydown", preventDefault: vi.fn() });
    await flush(0);
    expect(order).toEqual(['{"text":"l"}']);
    release();
    await flush(0);
    expect(order).toEqual(['{"text":"l"}', '{"text":"s!"}', '{"keys":["enter"]}']);
  });

  it("keeps a send error when a later read succeeds", async () => {
    vi.mocked(sendText).mockRejectedValue(new Error("nope"));
    view();
    await flush(0);
    term().data("x");
    await flush(300);
    expect(screen.getByRole("alert")).toHaveTextContent("Couldn't send input: nope");
  });

  it("a later successful send clears the send error", async () => {
    vi.mocked(sendText).mockResolvedValue(undefined).mockRejectedValueOnce(new Error("nope"));
    view();
    await flush(0);
    term().data("x");
    await flush(0);
    expect(screen.getByRole("alert")).toHaveTextContent("Couldn't send input: nope");
    term().data("y");
    await flush(0);
    expect(sendText).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("re-reads the screen as soon as a send finishes", async () => {
    view();
    await flush(0);
    expect(readScreen).toHaveBeenCalledTimes(1);
    term().data("x");
    await flush(0);
    expect(readScreen).toHaveBeenCalledTimes(2);
  });

  it("hands the mouse to the app while herdr has no scrollback for the pane", async () => {
    vi.mocked(readScreen).mockResolvedValue(shown("full-screen app", false, 0, 0));
    vi.mocked(scrollScreen).mockResolvedValue({ offset: 3, max: 50 });
    view();
    await flush(0);
    expect(term().writes).toContain(MOUSE_ON);
    expect(term().wheelHandler(wheel(-48))).toBe(true); // xterm reports it to the app
    await flush(0);
    expect(scrollScreen).not.toHaveBeenCalled();
    term().data("\x1b[<64;5;5M");
    await flush(0);
    expect(sendText).toHaveBeenCalledWith("w1:p1", "\x1b[<64;5;5M");

    vi.mocked(readScreen).mockResolvedValue(shown("shell", false, 0, 50));
    await flush(SCREEN_POLL_MS);
    expect(term().writes.filter((w: string) => w === MOUSE_OFF)).toHaveLength(1);
    expect(term().wheelHandler(wheel(-48))).toBe(false);
  });

  it("sends a paste as a paste and typing as raw text", async () => {
    view();
    fireEvent.paste(screen.getByRole("group"));
    term().data("pasted text");
    term().data("t");
    await flush(0);
    expect(sendInput).toHaveBeenCalledWith("w1:p1", { text: "pasted text" });
    expect(sendText).toHaveBeenCalledWith("w1:p1", "t");
    expect(sendText).toHaveBeenCalledTimes(1);
  });

  it("never merges mouse reports into one burst", async () => {
    let release!: () => void;
    vi.mocked(sendText).mockImplementationOnce(() => new Promise<void>((r) => (release = r)));
    view();
    const t = term();
    t.data("a");
    t.data("\x1b[<64;5;5M");
    t.data("\x1b[<64;5;5M");
    t.data("b");
    await flush(0);
    release();
    await flush(0);
    expect(vi.mocked(sendText).mock.calls.map(([, text]) => text)).toEqual(["a", "\x1b[<64;5;5M", "\x1b[<64;5;5M", "b"]);
  });

  it("Option+drag still selects locally", () => {
    view();
    expect(term().opts.macOptionClickForcesSelection).toBe(true);
  });

  it("wheel scrolls the pane through its history, clamped to it", async () => {
    let at = 0;
    vi.mocked(readScreen).mockImplementation(async () => shown("hello", false, at));
    vi.mocked(scrollScreen).mockImplementation(async (_p, offset) => ({ offset: (at = offset), max: 100 }));
    view();
    await flush(0);
    const up = wheel(-3 * WHEEL_PX_PER_LINE);
    expect(term().wheelHandler(up)).toBe(false);
    expect(up.preventDefault).toHaveBeenCalled();
    await flush(0);
    expect(scrollScreen).toHaveBeenLastCalledWith("w1:p1", 3);
    expect(readScreen).toHaveBeenCalledTimes(2); // re-read after scrolling
    term().wheelHandler(wheel(WHEEL_PX_PER_LINE));
    await flush(0);
    expect(scrollScreen).toHaveBeenLastCalledWith("w1:p1", 2);
    term().wheelHandler(wheel(-1000, { deltaMode: 1 }));
    await flush(0);
    expect(scrollScreen).toHaveBeenLastCalledWith("w1:p1", 100);
    term().wheelHandler(wheel(-1000, { deltaMode: 1 }));
    await flush(0);
    expect(scrollScreen).toHaveBeenCalledTimes(3); // already at the top
  });

  it("small wheel moves add up to a line", async () => {
    vi.mocked(scrollScreen).mockImplementation(async (_p, offset) => ({ offset, max: 100 }));
    view();
    await flush(0);
    term().wheelHandler(wheel(-WHEEL_PX_PER_LINE / 2));
    await flush(0);
    expect(scrollScreen).not.toHaveBeenCalled();
    term().wheelHandler(wheel(-WHEEL_PX_PER_LINE / 2));
    await flush(0);
    expect(scrollScreen).toHaveBeenCalledWith("w1:p1", 1);
  });

  it("leaves sideways wheel to the page", async () => {
    view();
    await flush(0);
    const e = wheel(1, { deltaX: 40 });
    expect(term().wheelHandler(e)).toBe(true);
    expect(e.preventDefault).not.toHaveBeenCalled();
  });

  it("typing while scrolled up jumps back to the bottom first", async () => {
    vi.mocked(readScreen).mockResolvedValue(shown("old", false, 10));
    const order: string[] = [];
    vi.mocked(scrollScreen).mockImplementation(async (_p, offset) => (order.push(`scroll ${offset}`), { offset, max: 100 }));
    vi.mocked(sendText).mockImplementation(async (_p, text) => void order.push(text));
    view();
    await flush(0);
    term().data("x");
    await flush(0);
    expect(order).toEqual(["scroll 0", "x"]);
  });

  it("scrolls the pane back to the bottom when the view goes away", async () => {
    vi.mocked(readScreen).mockResolvedValue(shown("old", false, 10));
    vi.mocked(scrollScreen).mockResolvedValue({ offset: 0, max: 100 });
    const { unmount } = view();
    await flush(0);
    unmount();
    expect(scrollScreen).toHaveBeenCalledWith("w1:p1", 0);
  });

  it("is accessible", async () => {
    vi.useRealTimers();
    const { container } = view();
    expect(screen.getByRole("group", { name: "Terminal for fix bug" })).toBeInTheDocument();
    expect(term().opts.screenReaderMode).toBe(true);
    expect(await axe(container)).toHaveNoViolations();
  });
});
