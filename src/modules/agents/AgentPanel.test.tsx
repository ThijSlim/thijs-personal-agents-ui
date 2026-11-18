import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "vitest-axe";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent, Conversation, UiState } from "../../core/types";
import { approve, deny, markSeen, readConversation, sendReply } from "../../core/ipc";
import { AgentPanel } from "./AgentPanel";

vi.mock("../../core/ipc", () => ({
  markSeen: vi.fn().mockResolvedValue(undefined),
  readConversation: vi.fn(),
  sendReply: vi.fn().mockResolvedValue(undefined),
  approve: vi.fn().mockResolvedValue(undefined),
  deny: vi.fn().mockResolvedValue(undefined),
}));

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
    // One answer per prompt: both buttons wait for the next read.
    expect(screen.getByRole("button", { name: "Deny" })).toBeDisabled();
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

  it("no axe violations", async () => {
    const { container } = renderPanel(blocked, agent("blocked"));
    await screen.findByRole("button", { name: "Approve" });
    expect(await axe(container)).toHaveNoViolations();
  });
});
