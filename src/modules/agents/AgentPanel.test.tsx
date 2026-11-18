import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "vitest-axe";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent, Conversation, UiState } from "../../core/types";
import { approve, deny, markSeen, readConversation, sendReply } from "../../core/ipc";
import { AgentPanel, POLL_MS } from "./AgentPanel";

vi.mock("../../core/ipc", () => ({
  markSeen: vi.fn().mockResolvedValue(undefined),
  readConversation: vi.fn(),
  sendReply: vi.fn().mockResolvedValue(undefined),
  approve: vi.fn().mockResolvedValue(undefined),
  deny: vi.fn().mockResolvedValue(undefined),
  readScreen: vi.fn().mockResolvedValue({ text: "", ended: false }),
  sendInput: vi.fn().mockResolvedValue(undefined),
}));
// jsdom has no canvas/matchMedia, which xterm needs.
const xt = vi.hoisted(() => ({ keyHandler: undefined as undefined | ((e: unknown) => boolean) }));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    options: Record<string, unknown> = {};
    open = vi.fn();
    resize = vi.fn();
    write = vi.fn();
    attachCustomKeyEventHandler = (f: (e: unknown) => boolean) => void (xt.keyHandler = f);
    onData = () => ({ dispose() {} });
    dispose = vi.fn();
  },
}));
vi.mock("@xterm/xterm/css/xterm.css", () => ({}));

const agent = (status: Agent["status"], seq = 1): Agent => ({
  paneId: "w1:p1", workspaceId: "w1", tabId: "w1:t1", agent: "claude", title: "fix bug", status, stateChangeSeq: seq,
});
const stateWith = (a: Agent | null): UiState => ({
  connection: "connected",
  spaces: [{ id: "w1", label: "alpha", agents: a ? [a] : [] }],
  needsYou: [],
});
const conv = (over: Partial<Conversation> = {}): Conversation => ({
  text: "❯ hello\n⏺ Hi there, how can I help?",
  permission: null,
  ended: false,
  ...over,
});
const blocked = conv({ permission: { question: "Bash command\ntouch probe.txt\nDo you want to proceed?" } });

const renderPanel = (c: Conversation, a: Agent | null = agent("idle")) => {
  vi.mocked(readConversation).mockResolvedValue(c);
  return render(<AgentPanel paneId="w1:p1" state={stateWith(a)} />);
};
const replyBox = () => screen.getByRole("textbox", { name: "Reply" });

beforeEach(() => vi.clearAllMocks());

describe("AgentPanel", () => {
  it("shows conversation text", async () => {
    renderPanel(conv());
    const log = await screen.findByRole("log");
    await waitFor(() => expect(log).toHaveTextContent("Hi there, how can I help?"));
    expect(log).toHaveAttribute("aria-live", "off");
    expect(readConversation).toHaveBeenCalledWith("w1:p1");
    expect(screen.getByRole("heading", { level: 1, name: "fix bug" })).toBeInTheDocument();
    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(screen.getByText("Idle")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Conversation" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Terminal" })).toBeInTheDocument();
  });

  it("shows Approve and Deny only when permission present", async () => {
    const { unmount } = renderPanel(conv());
    await waitFor(() => expect(screen.getByRole("log")).toHaveTextContent("Hi there"));
    expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Deny" })).not.toBeInTheDocument();
    unmount();

    const second = renderPanel(blocked, agent("blocked"));
    const approveButton = await screen.findByRole("button", { name: "Approve" });
    expect(screen.getByText(/touch probe\.txt/, { selector: ":not([role=log])" })).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Agent needs approval");
    await userEvent.click(approveButton);
    expect(approve).toHaveBeenCalledWith("w1:p1");
    // The single-answer guard is covered by the stale-read test below.
    second.unmount();

    renderPanel(blocked, agent("blocked"));
    await userEvent.click(await screen.findByRole("button", { name: "Deny" }));
    expect(deny).toHaveBeenCalledWith("w1:p1");
    expect(approve).toHaveBeenCalledTimes(1);
  });

  it("reply sends exact multi-line unicode text", async () => {
    renderPanel(conv());
    await userEvent.type(replyBox(), "line1{Shift>}{Enter}{/Shift}line2 — ✓ 日本");
    expect(replyBox()).toHaveValue("line1\nline2 — ✓ 日本");
    expect(sendReply).not.toHaveBeenCalled();
    await userEvent.keyboard("{Enter}");
    expect(sendReply).toHaveBeenCalledTimes(1);
    expect(sendReply).toHaveBeenCalledWith("w1:p1", "line1\nline2 — ✓ 日本");
    await waitFor(() => expect(replyBox()).toHaveValue(""));

    await userEvent.type(replyBox(), "  padded  ");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(sendReply).toHaveBeenLastCalledWith("w1:p1", "  padded  ");
  });

  it("reply disabled when empty or whitespace", async () => {
    renderPanel(conv());
    const send = screen.getByRole("button", { name: "Send" });
    expect(send).toBeDisabled();
    await userEvent.type(replyBox(), "  {Shift>}{Enter}{/Shift} ");
    expect(send).toBeDisabled();
    await userEvent.keyboard("{Enter}");
    expect(sendReply).not.toHaveBeenCalled();
    await userEvent.type(replyBox(), "x");
    expect(send).toBeEnabled();
  });

  it("ended session disables inputs and shows Session ended", async () => {
    renderPanel({ ...blocked, ended: true }, null);
    expect(await screen.findByText("Session ended")).toBeInTheDocument();
    expect(replyBox()).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Deny" })).toBeDisabled();
  });

  it("marks seen on open", async () => {
    const { rerender } = renderPanel(conv());
    expect(markSeen).toHaveBeenCalledWith("w1:p1");
    rerender(<AgentPanel paneId="w2:p2" state={stateWith(agent("idle"))} />);
    expect(markSeen).toHaveBeenLastCalledWith("w2:p2");
    await waitFor(() => expect(readConversation).toHaveBeenLastCalledWith("w2:p2"));
  });

  it("re-reads when this pane's state changes, at most every 500 ms", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    try {
      const { rerender } = renderPanel(conv());
      expect(readConversation).toHaveBeenCalledTimes(1);
      rerender(<AgentPanel paneId="w1:p1" state={stateWith(agent("working", 2))} />);
      rerender(<AgentPanel paneId="w1:p1" state={stateWith(agent("blocked", 3))} />);
      expect(readConversation).toHaveBeenCalledTimes(1);
      await act(async () => {
        vi.advanceTimersByTime(500);
      });
      expect(readConversation).toHaveBeenCalledTimes(2);
      // An unrelated change (another Space) doesn't re-read.
      const other = stateWith(agent("blocked", 3));
      other.spaces.push({ id: "w2", label: "beta", agents: [] });
      rerender(<AgentPanel paneId="w1:p1" state={other} />);
      await act(async () => {
        vi.advanceTimersByTime(1000);
      });
      expect(readConversation).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  describe("polling while working", () => {
    const tick = (ms: number) =>
      act(async () => {
        vi.advanceTimersByTime(ms);
      });
    const setVisibility = (v: DocumentVisibilityState) => {
      Object.defineProperty(document, "visibilityState", { configurable: true, get: () => v });
      document.dispatchEvent(new Event("visibilitychange"));
    };

    beforeEach(() => {
      vi.useFakeTimers({ shouldAdvanceTime: false });
      setVisibility("visible");
    });
    afterEach(() => {
      vi.useRealTimers();
      setVisibility("visible");
    });

    it("polls every POLL_MS while working on the Conversation tab", async () => {
      expect(POLL_MS).toBe(1500);
      renderPanel(conv(), agent("working"));
      expect(readConversation).toHaveBeenCalledTimes(1);
      await tick(POLL_MS);
      expect(readConversation).toHaveBeenCalledTimes(2);
      await tick(POLL_MS);
      expect(readConversation).toHaveBeenCalledTimes(3);
    });

    it("does not poll when idle or blocked", async () => {
      const { rerender } = renderPanel(conv(), agent("idle"));
      await tick(POLL_MS * 3);
      expect(readConversation).toHaveBeenCalledTimes(1);
      rerender(<AgentPanel paneId="w1:p1" state={stateWith(agent("blocked", 2))} />);
      await tick(POLL_MS * 3);
      expect(readConversation).toHaveBeenCalledTimes(2); // the status change itself
    });

    it("stops polling when working stops", async () => {
      const { rerender } = renderPanel(conv(), agent("working"));
      await tick(POLL_MS);
      expect(readConversation).toHaveBeenCalledTimes(2);
      rerender(<AgentPanel paneId="w1:p1" state={stateWith(agent("idle", 2))} />);
      await tick(POLL_MS * 3);
      expect(readConversation).toHaveBeenCalledTimes(3); // the status change itself
    });

    it("does not poll on the Terminal tab", async () => {
      renderPanel(conv(), agent("working"));
      fireEvent.mouseDown(screen.getByRole("tab", { name: "Terminal" }), { button: 0 });
      expect(screen.getByRole("tab", { name: "Terminal" })).toHaveAttribute("aria-selected", "true");
      await tick(POLL_MS * 3);
      expect(readConversation).toHaveBeenCalledTimes(1);
      fireEvent.mouseDown(screen.getByRole("tab", { name: "Conversation" }), { button: 0 });
      await tick(POLL_MS);
      expect(vi.mocked(readConversation).mock.calls.length).toBeGreaterThan(1);
    });

    it("does not poll while the window is hidden", async () => {
      renderPanel(conv(), agent("working"));
      act(() => setVisibility("hidden"));
      await tick(POLL_MS * 3);
      expect(readConversation).toHaveBeenCalledTimes(1);
      act(() => setVisibility("visible"));
      await tick(POLL_MS);
      expect(vi.mocked(readConversation).mock.calls.length).toBeGreaterThan(1);
    });

    it("stops polling on unmount", async () => {
      const { unmount } = renderPanel(conv(), agent("working"));
      await tick(POLL_MS);
      expect(readConversation).toHaveBeenCalledTimes(2);
      unmount();
      await tick(POLL_MS * 3);
      expect(readConversation).toHaveBeenCalledTimes(2);
    });
  });

  it("a read that started before Approve doesn't re-enable the buttons; the read after it does", async () => {
    const pending: Array<(c: Conversation) => void> = [];
    vi.mocked(readConversation).mockImplementation(() => new Promise((r) => pending.push(r)));
    const { rerender } = render(<AgentPanel paneId="w1:p1" state={stateWith(agent("blocked"))} />);
    await act(async () => pending.shift()!(blocked));
    // A state change starts a read that is still in flight when Approve is pressed.
    rerender(<AgentPanel paneId="w1:p1" state={stateWith(agent("blocked", 2))} />);
    await waitFor(() => expect(pending).toHaveLength(1));
    const stale = pending.shift()!;
    await userEvent.click(screen.getByRole("button", { name: "Approve" }));
    expect(approve).toHaveBeenCalledTimes(1);
    // The press resolving triggers a fresh read.
    await waitFor(() => expect(pending).toHaveLength(1));
    // The stale read (started before the press) must not re-enable the buttons.
    await act(async () => stale(blocked));
    expect(screen.getByRole("button", { name: "Approve" })).toBeDisabled();
    // The read started after the press does.
    await act(async () => pending.shift()!(blocked));
    expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled();
  });

  it("read failures are announced", async () => {
    vi.mocked(readConversation).mockRejectedValue(new Error("boom"));
    render(<AgentPanel paneId="w1:p1" state={stateWith(agent("idle"))} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't read the conversation: boom");
  });

  it("text typed while a reply is sending is kept", async () => {
    let finish!: () => void;
    vi.mocked(sendReply).mockImplementation(() => new Promise<void>((r) => (finish = r)));
    renderPanel(conv());
    await userEvent.type(replyBox(), "first{Enter}");
    expect(sendReply).toHaveBeenCalledWith("w1:p1", "first");
    await userEvent.type(replyBox(), " more");
    await act(async () => finish());
    expect(replyBox()).toHaveValue(" more");
  });

  it("Cmd+Escape in the terminal focuses the Terminal tab", async () => {
    renderPanel(conv());
    await screen.findByRole("log");
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Terminal" }), { button: 0 });
    await screen.findByRole("group", { name: "Terminal for fix bug" });
    xt.keyHandler!({ key: "Escape", metaKey: true, type: "keydown", preventDefault() {} });
    expect(screen.getByRole("tab", { name: "Terminal" })).toHaveFocus();
  });

  it("no axe violations with the Terminal tab active", async () => {
    const { container } = renderPanel(conv());
    await screen.findByRole("log");
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Terminal" }), { button: 0 });
    expect(await screen.findByRole("group", { name: "Terminal for fix bug" })).toBeInTheDocument();
    expect(await axe(container)).toHaveNoViolations();
  });

  it("no axe violations", async () => {
    const { container } = renderPanel(blocked, agent("blocked"));
    await screen.findByRole("button", { name: "Approve" });
    expect(await axe(container)).toHaveNoViolations();
  });
});
