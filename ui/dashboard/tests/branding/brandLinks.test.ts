import { describe, expect, it } from "vitest";
import {
  BETTERLITE_DOCS_URL,
  BETTERLITE_RELEASES_URL,
  BETTERLITE_SOURCE_URL,
  BETTERLITE_SUPPORT_URL,
  LITELLM_LICENSE_NOTICE,
  UPSTREAM_LITELLM_DOCS_LABEL,
} from "@/components/branding/brandLinks";

describe("brandLinks", () => {
  it("points every shared product link at betterlite's own repository", () => {
    expect(BETTERLITE_SOURCE_URL).toBe("https://github.com/betterlite-ai/betterlite");
    expect(BETTERLITE_DOCS_URL).toBe("https://github.com/betterlite-ai/betterlite#readme");
    expect(BETTERLITE_SUPPORT_URL).toBe("https://github.com/betterlite-ai/betterlite/issues");
    expect(BETTERLITE_RELEASES_URL).toBe("https://github.com/betterlite-ai/betterlite/releases");
  });

  it("names the bundled Berri AI notice relative to the UI base", () => {
    expect(LITELLM_LICENSE_NOTICE).toBe("notices/LiteLLM-LICENSE.txt");
  });

  it("labels a retained upstream reference as LiteLLM's documentation", () => {
    expect(UPSTREAM_LITELLM_DOCS_LABEL).toBe("Upstream LiteLLM documentation");
  });
});
