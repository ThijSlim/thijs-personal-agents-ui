import { act, render, screen } from "@testing-library/react";
import { axe } from "vitest-axe";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readScreen, sendInput } from "../../core/ipc";
import { TerminalView, keyToHerdr } from "./TerminalView";

vi.mock("../../core/ipc", () => ({ readScreen: vi.fn(), sendInput: vi.fn().mockResolvedValue(undefined) }));

const h = vi.hoisted(() => ({ terms: [] as any[] }));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    writes: string[] = [];
    disposed = false;
    onData = (cb: (d: string) => void) => ((this as any).data = cb, { dispose() {} });
    keyHandler?: (e: KeyboardEvent) => boolean;
    attachCustomKeyEventHandler = (f: (e: KeyboardEvent) => boolean) => void (this.keyHandler = f);
    constructor(public opts: any) {
      h.terms.push(this);
    }
    options: any = {};
    open = vi.fn();
    resize = vi.fn();
    write = (s: string) => void this.writes.push(s);
    focus = vi.fn();
    dispose = () => void (this.disposed = true);
  },
}));
vi.mock("@xterm/xterm/css/xterm.css", () => ({}));

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

const flush = async (ms = 0) => act(async () => void (await vi.advanceTimersByTimeAsync(ms)));
const term = () => h.terms[h.terms.length - 1];

describe("TerminalView", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    h.terms.length = 0;
    vi.mocked(readScreen).mockResolvedValue({ text: "hello", ended: false });
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
  });
  afterEach(() => vi.useRealTimers());

  const view = (over: Partial<Parameters<typeof TerminalView>[0]> = {}) =>
    render(<TerminalView paneId="w1:p1" label="fix bug" active {...over} />);

  it("polls only while visible", async () => {
    const { rerender } = view({ active: false });
    await flush(1000);
    expect(readScreen).not.toHaveBeenCalled();
    rerender(<TerminalView paneId="w1:p1" label="fix bug" active />);
    await flush(0);
    expect(readScreen).toHaveBeenCalledTimes(1); // immediately
    await flush(500);
    expect(readScreen).toHaveBeenCalledTimes(3);
    rerender(<TerminalView paneId="w1:p1" label="fix bug" active={false} />);
    await flush(1000);
    expect(readScreen).toHaveBeenCalledTimes(3);
  });

  it("pauses while the window is blurred and resumes on focus", async () => {
    view();
    await flush(0);
    expect(readScreen).toHaveBeenCalledTimes(1);
    act(() => void window.dispatchEvent(new Event("blur")));
    await flush(1000);
    expect(readScreen).toHaveBeenCalledTimes(1);
    act(() => void window.dispatchEvent(new Event("focus")));
    await flush(0);
    expect(readScreen).toHaveBeenCalledTimes(2);
  });

  it("does not poll when the window starts unfocused", async () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    view();
    await flush(1000);
    expect(readScreen).not.toHaveBeenCalled();
  });

  it("redraws only when the text changed", async () => {
    view();
    await flush(0);
    expect(term().writes).toHaveLength(1);
    expect(term().writes[0]).toBe("\x1b[0m\x1b[H\x1b[2Jhello");
    await flush(500);
    expect(readScreen).toHaveBeenCalledTimes(3);
    expect(term().writes).toHaveLength(1);
    vi.mocked(readScreen).mockResolvedValue({ text: "a\nb", ended: false });
    await flush(250);
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

  it("forwards special keys as keys and printable text as text", async () => {
    view();
    const t = term();
    const ev = (k: string, o = {}) => ({ ...key(k, o), preventDefault: vi.fn(), type: "keydown" }) as any;
    expect(t.keyHandler(ev("Enter"))).toBe(false);
    expect(sendInput).toHaveBeenCalledWith("w1:p1", { keys: ["enter"] });
    expect(t.keyHandler(ev("c", { metaKey: true }))).toBe(true);
    expect(t.keyHandler(ev("a"))).toBe(true);
    t.data("ab");
    await flush(0);
    expect(sendInput).toHaveBeenCalledWith("w1:p1", { text: "ab" });
    expect(sendInput).toHaveBeenCalledTimes(2);
  });

  it("ended disables input and says so", async () => {
    vi.mocked(readScreen).mockResolvedValue({ text: "bye", ended: true });
    view();
    await flush(0);
    expect(screen.getByRole("status")).toHaveTextContent("Session ended");
    expect(screen.getByRole("group")).toHaveAttribute("aria-disabled", "true");
    const t = term();
    expect(t.options.disableStdin).toBe(true);
    t.data("x");
    t.keyHandler({ ...key("Enter"), type: "keydown", preventDefault: vi.fn() });
    expect(sendInput).not.toHaveBeenCalled();
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
    vi.mocked(sendInput).mockImplementation(async (_p, i) => {
      inFlight++;
      expect(inFlight).toBe(1);
      order.push(JSON.stringify(i));
      if (order.length === 1) await new Promise<void>((r) => (release = r));
      inFlight--;
    });
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
    vi.mocked(sendInput).mockRejectedValue(new Error("nope"));
    view();
    await flush(0);
    term().data("x");
    await flush(300);
    expect(screen.getByRole("alert")).toHaveTextContent("Couldn't send input: nope");
  });

  it("is accessible", async () => {
    vi.useRealTimers();
    const { container } = view();
    expect(screen.getByRole("group", { name: "Terminal for fix bug" })).toBeInTheDocument();
    expect(term().opts.screenReaderMode).toBe(true);
    expect(await axe(container)).toHaveNoViolations();
  });
});
