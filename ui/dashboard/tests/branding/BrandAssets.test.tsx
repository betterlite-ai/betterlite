import { describe, expect, it } from "vitest";
import { render, screen } from "../test-utils";
import { BRAND_WORDMARK_SRC } from "@/components/branding/BrandWordmark";
import { guardrailLogoMap, getGuardrailLogo } from "@/app/(dashboard)/guardrails/_components/guardrail_info_helpers";
import { LITELLM_CONTENT_FILTER_CARDS } from "@/app/(dashboard)/guardrails/_components/guardrail_garden_data";
import { Logo } from "@/components/molecules/logo/Logo";
import { logoTreatmentFor } from "@/lib/logoTreatments";

const LITELLM_OWNED_ENTRIES = ["LiteLLM Content Filter", "LiteLLM LLM as a Judge", "Hide Secrets", "Decision Model"];

describe("guardrail logos", () => {
  it("renders the wordmark for every guardrail LiteLLM owns", () => {
    for (const name of LITELLM_OWNED_ENTRIES) {
      expect(guardrailLogoMap[name as keyof typeof guardrailLogoMap]).toBe(BRAND_WORDMARK_SRC);
    }
  });

  it("keeps each provider's own logo unchanged", () => {
    expect(guardrailLogoMap["Presidio PII"]).toContain("microsoft_azure.svg");
    expect(getGuardrailLogo("OpenAI Moderation")).toContain("openai_small.svg");
    expect(getGuardrailLogo("Lakera")).not.toBe(BRAND_WORDMARK_SRC);
  });

  it("leaves no guardrail pointing at the deleted LiteLLM monogram", () => {
    expect(Object.values(guardrailLogoMap).filter((logo) => logo.includes("litellm_monogram"))).toEqual([]);
  });

  it("uses the wordmark sentinel for every content filter card", () => {
    for (const card of LITELLM_CONTENT_FILTER_CARDS) {
      expect(card.logo).toBe(BRAND_WORDMARK_SRC);
    }
  });
});

describe("Logo", () => {
  it("draws the text wordmark for the sentinel instead of an image", () => {
    render(<Logo src={BRAND_WORDMARK_SRC} label="LiteLLM Content Filter" />);

    expect(screen.getByText("b")).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: "LiteLLM Content Filter logo" })).not.toBeInTheDocument();
  });

  it("still draws a provider image wherever a provider logo exists", () => {
    render(<Logo src="/ui/assets/logos/openai_small.svg" label="OpenAI" />);

    expect(screen.getByRole("img", { name: "OpenAI logo" })).toHaveAttribute(
      "src",
      "/ui/assets/logos/openai_small.svg",
    );
  });
});

describe("logoTreatments", () => {
  it("no longer treats the deleted LiteLLM monogram as an invertible logo", () => {
    expect(
      logoTreatmentFor("/litellm-asset-prefix/_next/static/media/litellm_monogram.16r2_rdj4dq8r.svg"),
    ).toBeUndefined();
  });

  it("still treats provider marks that remain bundled", () => {
    expect(logoTreatmentFor("/ui/assets/logos/github.svg")).toBe("invert");
    expect(logoTreatmentFor("/ui/assets/logos/aws.svg")).toBe("plate");
  });
});
