import { describe, expect, it } from "vitest";
import { maxButton } from "../src/titlebar";

describe("maxButton (what the middle window button says and shows)", () => {
  it("a normal window offers to fill the screen", () => {
    expect(maxButton(false)).toEqual({ label: "Maximize", icon: "max" });
  });

  it("a window that fills the screen offers to go back to its size", () => {
    expect(maxButton(true)).toEqual({ label: "Restore", icon: "restore" });
  });
});
