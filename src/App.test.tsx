import { render, screen } from "@testing-library/react";
import { axe } from "vitest-axe";
import { describe, expect, it, vi } from "vitest";
import App from "./App";

vi.mock("./core/ipc", () => ({
  getState: vi.fn().mockResolvedValue({ connection: "connecting", spaces: [], needsYou: [] }),
  onStateChanged: vi.fn().mockResolvedValue(() => {}),
}));

describe("App", () => {
  it("renders the agents navigation with no a11y violations", async () => {
    const { container } = render(<App />);
    expect(screen.getByRole("navigation", { name: "Agents" })).toBeInTheDocument();
    expect(await axe(container)).toHaveNoViolations();
  });
});
