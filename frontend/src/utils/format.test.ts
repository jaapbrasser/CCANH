import { errorCategoryLabel, relativeTime } from "./format";

describe("relativeTime", () => {
  const now = Date.parse("2026-11-18T12:00:00Z");
  it.each([
    [null, "—"],
    ["not a date", "—"],
    ["2026-11-18T11:59:58Z", "just now"],
    ["2026-11-18T11:59:30Z", "30 s ago"],
    ["2026-11-18T11:55:00Z", "5 min ago"],
    ["2026-11-18T09:00:00Z", "3 h ago"],
  ])("%s → %s", (iso, expected) => {
    expect(relativeTime(iso, now)).toBe(expected);
  });
});

it("labels error categories", () => {
  expect(errorCategoryLabel("auth_failed")).toBe("Authentication failed");
  expect(errorCategoryLabel(null)).toBeNull();
});
