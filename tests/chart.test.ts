import { describe, expect, it } from "vitest";
import { place, typical } from "../src/chart";

describe("typical (the level the lead-in is drawn at: the middle reading so far)", () => {
  it("ignores the spike of the app's own start-up", () => {
    expect(typical([57, 20, 10, 8, 7, 7, 8])).toBe(8);
  });
  it("one reading is its own level", () => {
    expect(typical([41])).toBe(41);
  });
  it("an even count takes the middle two", () => {
    expect(typical([10, 20, 30, 40])).toBe(25);
  });
  it("nothing measured is zero, and bad numbers are left out", () => {
    expect(typical([])).toBe(0);
    expect(typical([Number.NaN, 5, 7])).toBe(6);
  });
  it("does not reorder the readings it was given", () => {
    const readings = [3, 1, 2];
    typical(readings);
    expect(readings).toEqual([3, 1, 2]);
  });
});

describe("place (where a line sits on a graph, and how much of the graph has no readings yet)", () => {
  it("a full history covers the whole width: no lead-in", () => {
    expect(place(120, 120, 600)).toMatchObject({ n: 120, from: 0, x0: 0, lead: 0 });
  });

  it("more readings than the graph holds: only the newest are drawn", () => {
    expect(place(150, 120, 600)).toMatchObject({ n: 120, from: 30, x0: 0, lead: 0 });
  });

  it("a short history sits at the right edge, and the empty part before it is the lead-in", () => {
    const p = place(61, 121, 600);
    expect(p.n).toBe(61);
    expect(p.x0).toBeCloseTo(300, 5);
    expect(p.lead).toBeCloseTo(300, 5);
  });

  it("the very first reading: the lead-in spans the whole graph, so it is never an empty box", () => {
    const p = place(1, 120, 600);
    expect(p).toMatchObject({ n: 1, from: 0 });
    expect(p.x0).toBeCloseTo(600, 5);
    expect(p.lead).toBeCloseTo(600, 5);
  });

  it("no reading at all: nothing to draw and nothing to carry back", () => {
    expect(place(0, 120, 600)).toMatchObject({ n: 0, lead: 0 });
  });

  it("a graph that stands for one point does not divide by zero", () => {
    const p = place(1, 1, 600);
    expect(Number.isFinite(p.step)).toBe(true);
    expect(p.lead).toBe(600);
  });
});
