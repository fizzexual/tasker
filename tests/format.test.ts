import { describe, expect, it } from "vitest";
import { bytes, count, driverDate, duration, niceCeil, percent, percent0, rate, rateFull } from "../src/format";

describe("bytes", () => {
  it("picks the unit that keeps the number short", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(512)).toBe("512 B");
    expect(bytes(1024)).toBe("1.0 KB");
    expect(bytes(1536)).toBe("1.5 KB");
    expect(bytes(150 * 1024 * 1024)).toBe("150 MB");
    expect(bytes(3.25 * 1024 ** 3)).toBe("3.3 GB");
  });
  it("never shows 1024 of a unit", () => {
    expect(bytes(1024 * 1024 - 1)).toBe("1.0 MB");
    expect(bytes(1023.99 * 1024 ** 2)).toBe("1.0 GB");
  });
  it("treats bad input as zero instead of printing NaN", () => {
    expect(bytes(NaN)).toBe("0 B");
    expect(bytes(-5)).toBe("0 B");
    expect(bytes(Infinity)).toBe("0 B");
  });
  it("stops at the largest unit", () => {
    expect(bytes(5 * 1024 ** 6)).toBe("5120 PB");
  });
});

describe("rate", () => {
  it("is quiet below 1 KB/s", () => {
    expect(rate(0)).toBe("0");
    expect(rate(1023)).toBe("0");
    expect(rate(NaN)).toBe("0");
  });
  it("shows a unit per second above that", () => {
    expect(rate(2048)).toBe("2.0 KB/s");
    expect(rateFull(0)).toBe("0 B/s");
    expect(rateFull(500)).toBe("500 B/s");
  });
});

describe("percent", () => {
  it("shows one decimal", () => {
    expect(percent(0)).toBe("0.0%");
    expect(percent(12.345)).toBe("12.3%");
    expect(percent(100)).toBe("100.0%");
  });
  it("does not call a small real load zero", () => {
    expect(percent(0.02)).toBe("<0.1%");
  });
  it("never goes above 100 or below 0", () => {
    expect(percent(140)).toBe("100.0%");
    expect(percent(-3)).toBe("0.0%");
    expect(percent(NaN)).toBe("0.0%");
  });
  it("whole-number form", () => {
    expect(percent0(49.6)).toBe("50%");
    expect(percent0(0)).toBe("0%");
    expect(percent0(250)).toBe("100%");
    expect(percent(5, 0)).toBe("5%");
  });
});

describe("duration", () => {
  it("formats as a clock, with days when needed", () => {
    expect(duration(0)).toBe("00:00:00");
    expect(duration(3725)).toBe("01:02:05");
    expect(duration(93784)).toBe("1d 02:03:04");
  });
  it("shows a dash for a nonsense value", () => {
    expect(duration(-1)).toBe("–");
    expect(duration(NaN)).toBe("–");
  });
});

describe("count", () => {
  it("groups thousands", () => {
    expect(count(1234567)).toBe("1,234,567");
    expect(count(NaN)).toBe("–");
  });
});

describe("niceCeil (top of a speed graph)", () => {
  it("never goes below 100 KB/s so an idle line stays flat at the bottom", () => {
    expect(niceCeil(0)).toBe(100 * 1024);
    expect(niceCeil(5000)).toBe(100 * 1024);
    expect(niceCeil(NaN)).toBe(100 * 1024);
  });
  it("rounds up to a round number at or above the peak", () => {
    expect(niceCeil(150 * 1024)).toBe(200 * 1024);
    expect(niceCeil(3.2 * 1024 ** 2)).toBe(5 * 1024 ** 2);
    expect(niceCeil(5 * 1024 ** 2)).toBe(5 * 1024 ** 2);
    expect(niceCeil(700 * 1024 ** 2)).toBe(1024 ** 3);
  });
  it("is always at or above the value", () => {
    for (const v of [1, 99_999, 1_048_577, 9e8, 3e11]) expect(niceCeil(v)).toBeGreaterThanOrEqual(v);
  });
});

describe("driverDate", () => {
  it("turns Windows' month-day-year into year-month-day", () => {
    expect(driverDate("10-18-2026")).toBe("2026-10-18");
    expect(driverDate("3-5-2024")).toBe("2024-03-05");
    expect(driverDate(" 12-31-2025 ")).toBe("2025-12-31");
  });
  it("leaves anything it does not recognise as it came", () => {
    expect(driverDate("")).toBe("");
    expect(driverDate("2026-10-18")).toBe("2026-10-18");
    expect(driverDate("13-40-2026")).toBe("13-40-2026");
    expect(driverDate("unknown")).toBe("unknown");
  });
});
