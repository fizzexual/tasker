import { describe, expect, it } from "vitest";
import { maxLabel } from "../src/titlebar";

describe("maxLabel (what the green window button says it will do)", () => {
  it("a normal window offers to fill the screen", () => {
    expect(maxLabel(false)).toBe("Maximize");
  });

  it("a window that fills the screen offers to go back to its size", () => {
    expect(maxLabel(true)).toBe("Restore");
  });
});
