import { describe, expect, it } from "vitest";

import { resolveApiBase } from "@/lib/http/resolveApiBase";

describe("same-origin production build", () => {
  it("leaves the API base empty when no override is built in", () => {
    expect(resolveApiBase({ explicitBase: undefined })).toBe("");
  });

  it("uses an explicit override without its trailing slash", () => {
    expect(resolveApiBase({ explicitBase: "https://api.example.test/" })).toBe("https://api.example.test");
  });
});
