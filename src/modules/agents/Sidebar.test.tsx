import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { axe } from "vitest-axe";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Agent, UiState } from "../../core/types";
import { moveSpace, markSeen } from "../../core/ipc";
import { Sidebar } from "./Sidebar";

vi.mock("../../core/ipc", () => ({
  moveSpace: vi.fn().mockResolvedValue(undefined),
  markSeen: vi.fn().mockResolvedValue(undefined),
}));

const agent = (paneId: string, workspaceId: string, status: Agent["status"], title: string | null): Agent => ({
  paneId, workspaceId, tabId: "t", agent: "claude", title, status, stateChangeSeq: 1,
});

const connected: UiState = {
  connection: "connected",
  spaces: [
    { id: "w1", label: "alpha", agents: [agent("w1:p1", "w1", "blocked", "fix bug")] },
    { id: "w2", label: "beta", agents: [agent("w2:p2", "w2", "done", "write docs"), agent("w2:p3", "w2", "working", null)] },
  ],
  needsYou: [
    { paneId: "w1:p1", workspaceId: "w1", spaceLabel: "alpha", title: "fix bug", status: "blocked" },
    { paneId: "w2:p2", workspaceId: "w2", spaceLabel: "beta", title: "write docs", status: "done" },
  ],
};
const spaceItems = () =>
  Array.from(screen.getByRole("list", { name: "Spaces" }).querySelectorAll<HTMLElement>(":scope > li"));
const spaceButton = (name: string) =>
  within(screen.getByRole("list", { name: "Spaces" })).getByRole("button", { name });
const notRunning: UiState = { connection: "notRunning", spaces: [], needsYou: [] };

beforeEach(() => vi.clearAllMocks());

describe("Sidebar", () => {
  it("shows needs-you section with count and items", () => {
    render(<Sidebar state={connected} selection={null} onSelect={() => {}} />);
    const section = screen.getByRole("region", { name: "Needs you (2)" });
    expect(within(section).getByText("Blocked")).toBeInTheDocument();
    expect(within(section).getByText("Done")).toBeInTheDocument();
  });

  it("lists spaces in given order with agent state labels", () => {
    render(<Sidebar state={connected} selection={null} onSelect={() => {}} />);
    const items = spaceItems();
    expect(items.map((i) => within(i).getByText(/^(alpha|beta)$/).textContent)).toEqual(["alpha", "beta"]);
    expect(within(items[1]).getByText("Working")).toBeInTheDocument();
    expect(within(items[1]).getByText("Done")).toBeInTheDocument();
  });

  it("selecting an item calls onSelect and markSeen", async () => {
    const onSelect = vi.fn();
    render(<Sidebar state={connected} selection={null} onSelect={onSelect} />);
    const section = screen.getByRole("region", { name: "Needs you (2)" });
    await userEvent.click(within(section).getByRole("button", { name: /write docs/ }));
    expect(onSelect).toHaveBeenCalledWith({ module: "agents", id: "w2:p2" });
    expect(markSeen).toHaveBeenCalledWith("w2:p2");
  });

  it("alt+arrow moves focused space", async () => {
    render(<Sidebar state={connected} selection={null} onSelect={() => {}} />);
    const first = within(screen.getByRole("list", { name: "Spaces" })).getByRole("button", { name: "alpha" });
    first.focus();
    await userEvent.keyboard("{Alt>}{ArrowUp}{/Alt}");
    expect(moveSpace).not.toHaveBeenCalled();
    await userEvent.keyboard("{Alt>}{ArrowDown}{/Alt}");
    expect(moveSpace).toHaveBeenCalledWith("w1", 1);
  });

  it("announces the move after the state update arrives", async () => {
    const { rerender } = render(<Sidebar state={connected} selection={null} onSelect={() => {}} />);
    within(screen.getByRole("list", { name: "Spaces" })).getByRole("button", { name: "alpha" }).focus();
    await userEvent.keyboard("{Alt>}{ArrowDown}{/Alt}");
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("");
    rerender(<Sidebar state={{ ...connected, spaces: [connected.spaces[1], connected.spaces[0]] }} selection={null} onSelect={() => {}} />);
    expect(status).toHaveTextContent("Moved alpha to position 2 of 2");
  });

  it("does not announce for unrelated state changes while a move is pending", async () => {
    const mk = (spaces: UiState["spaces"]): UiState => ({ ...connected, spaces });
    const props = { selection: null, onSelect: () => {} };
    const { rerender } = render(<Sidebar state={connected} {...props} />);
    spaceButton("alpha").focus();
    await userEvent.keyboard("{Alt>}{ArrowDown}{/Alt}");
    const status = screen.getByRole("status");
    const changed = [
      { ...connected.spaces[0], agents: [agent("w1:p1", "w1", "working", "fix bug")] },
      connected.spaces[1],
    ];
    rerender(<Sidebar state={mk(changed)} {...props} />);
    expect(status).toHaveTextContent("");
    rerender(<Sidebar state={mk([changed[1], changed[0]])} {...props} />);
    expect(status).toHaveTextContent("Moved alpha to position 2 of 2");
    expect(spaceButton("alpha")).toHaveFocus();
  });

  it("announces a failed move", async () => {
    vi.mocked(moveSpace).mockRejectedValueOnce("herdr isn't running");
    render(<Sidebar state={connected} selection={null} onSelect={() => {}} />);
    spaceButton("alpha").focus();
    await userEvent.keyboard("{Alt>}{ArrowDown}{/Alt}");
    expect(await screen.findByText("Couldn't move alpha: herdr isn't running")).toBeInTheDocument();
  });

  it("re-announces identical messages", async () => {
    vi.mocked(moveSpace).mockRejectedValue("boom");
    render(<Sidebar state={connected} selection={null} onSelect={() => {}} />);
    const status = screen.getByRole("status");
    spaceButton("alpha").focus();
    await userEvent.keyboard("{Alt>}{ArrowDown}{/Alt}");
    await screen.findByText("Couldn't move alpha: boom");
    const first = status.textContent;
    await userEvent.keyboard("{Alt>}{ArrowDown}{/Alt}");
    await vi.waitFor(() => expect(status.textContent).not.toBe(first));
    expect(status).toHaveTextContent("Couldn't move alpha: boom");
    vi.mocked(moveSpace).mockResolvedValue(undefined);
  });

  it("clicking a space selects nothing", async () => {
    const onSelect = vi.fn();
    render(<Sidebar state={connected} selection={null} onSelect={onSelect} />);
    await userEvent.click(spaceButton("alpha"));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("drag and drop reorders", () => {
    render(<Sidebar state={connected} selection={null} onSelect={() => {}} />);
    const items = spaceItems();
    const dt = { setData: vi.fn(), getData: vi.fn(), effectAllowed: "", dropEffect: "" };
    fireEvent.dragStart(items[0], { dataTransfer: dt });
    fireEvent.dragOver(items[1], { dataTransfer: dt });
    fireEvent.drop(items[1], { dataTransfer: dt });
    expect(moveSpace).toHaveBeenCalledWith("w1", 1);
  });

  it("shows not-running banner", () => {
    render(<Sidebar state={notRunning} selection={null} onSelect={() => {}} />);
    expect(screen.getByText(/herdr isn't running/)).toBeInTheDocument();
    expect(screen.getByText("herdr", { selector: "code" })).toBeInTheDocument();
    expect(screen.queryByText(/Run /)).not.toBeInTheDocument();
    expect(screen.getByText(/this app reconnects automatically/)).toBeInTheDocument();
  });

  it("has no axe violations", async () => {
    const a = render(<Sidebar state={connected} selection={{ module: "agents", id: "w1:p1" }} onSelect={() => {}} />);
    expect(await axe(a.container)).toHaveNoViolations();
    a.unmount();
    const b = render(<Sidebar state={notRunning} selection={null} onSelect={() => {}} />);
    expect(await axe(b.container)).toHaveNoViolations();
  });
});
