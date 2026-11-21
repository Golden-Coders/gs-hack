import { describe, expect, it } from "vitest";
import { formatRedactedKinds } from "./Runbooks";

describe("formatRedactedKinds", () => {
  it("formatuje liczniki zredagowanych typów i pusty obiekt", () => {
    expect(formatRedactedKinds({})).toBe("");
    expect(formatRedactedKinds({ url: 2, ip: 1, email: 1 })).toBe("url: 2, ip: 1, email: 1");
  });
});
