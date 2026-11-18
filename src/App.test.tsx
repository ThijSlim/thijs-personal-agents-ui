import { act, render, screen } from "@testing-library/react";
import { axe } from "vitest-axe";
import { describe, expect, it, vi } from "vitest";
import App from "./App";
import * as ipc from "./core/ipc";
import type { UiState } from "./core/types";

const state: UiState = {
  connection: "connected",
  spaces: [{ id: "w1", label: "alpha", agents: [] }],
  needsYou: [],
};

vi.mock("./core/ipc", () => ({
  useUiState: vi.fn(() => state),
  onSelectAgent: vi.fn().mockResolvedValue(() => {}),
  markSeen: vi.fn().mockResolvedValue(undefined),
  moveSpace: vi.fn().mockResolvedValue(undefined),
}));

describe("App", () => {
  it("renders the agents navigation with no a11y violations", async () => {
    const { container } = render(<App />);
    expect(screen.getByRole("navigation", { name: "Agents" })).toBeInTheDocument();
    expect(await axe(container)).toHaveNoViolations();
  });

  it("selects the agent from a select-agent event", async () => {
    render(<App />);
    const cb = vi.mocked(ipc.onSelectAgent).mock.calls[0][0];
    act(() => cb("w1:p9"));
    expect(await screen.findByText("Selected: w1:p9")).toBeInTheDocument();
  });
});
