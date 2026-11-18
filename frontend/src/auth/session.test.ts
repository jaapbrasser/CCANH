import { safeNext } from "./session";

describe("safeNext", () => {
  it.each([
    [null, "/"],
    ["", "/"],
    ["/users?page=2", "/users?page=2"],
    ["/api/docs/", "/api/docs/"],
    ["https://evil.example", "/"],
    ["//evil.example", "/"],
    ["/\\evil.example", "/"],
    ["javascript:alert(1)", "/"],
  ])("safeNext(%j) = %j", (input, expected) => {
    expect(safeNext(input)).toBe(expected);
  });
});
