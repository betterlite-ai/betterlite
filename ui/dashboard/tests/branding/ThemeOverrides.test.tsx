import { ThemeProvider as NextThemesProvider } from "next-themes";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "../test-utils";
import { BrandLogo } from "@/components/branding/BrandLogo";
import { ThemeProvider } from "@/contexts/ThemeContext";

vi.mock("@/components/networking", () => ({
  getProxyBaseUrl: () => "http://proxy.test:4000",
}));

const transport = vi.fn<typeof fetch>();

const themeResponse = (values: Record<string, unknown>) =>
  new Response(JSON.stringify({ values }), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });

const renderThemed = (dark = false) => {
  const tree = (
    <ThemeProvider accessToken="test-token">
      <BrandLogo />
    </ThemeProvider>
  );
  return render(
    dark ? (
      <NextThemesProvider attribute="class" defaultTheme="dark">
        {tree}
      </NextThemesProvider>
    ) : (
      tree
    ),
  );
};

describe("operator theme overrides", () => {
  beforeEach(() => {
    transport.mockReset();
    vi.stubGlobal("fetch", transport);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    document.head.querySelectorAll("link[rel*='icon']").forEach((link) => link.remove());
  });

  it("reads the existing theme settings endpoint and wins with the configured logo", async () => {
    transport.mockResolvedValue(
      themeResponse({
        logo_url: "https://cdn.test/logo.png",
        logo_url_dark: "https://cdn.test/logo-dark.png",
        favicon_url: "https://cdn.test/favicon.ico",
      }),
    );

    renderThemed();

    await waitFor(() => {
      expect(transport).toHaveBeenCalledWith("http://proxy.test:4000/get/ui_theme_settings", expect.anything());
    });
    await waitFor(() => {
      expect(screen.getByRole("img", { name: "betterlite" })).toHaveAttribute("src", "https://cdn.test/logo.png");
    });
  });

  it("uses the configured dark logo under a dark theme", async () => {
    transport.mockResolvedValue(
      themeResponse({
        logo_url: "https://cdn.test/logo.png",
        logo_url_dark: "https://cdn.test/logo-dark.png",
      }),
    );

    renderThemed(true);

    await waitFor(() => {
      expect(screen.getByRole("img", { name: "betterlite" })).toHaveAttribute("src", "https://cdn.test/logo-dark.png");
    });
  });

  it("applies a configured favicon to the document", async () => {
    transport.mockResolvedValue(themeResponse({ favicon_url: "https://cdn.test/favicon.ico" }));

    renderThemed();

    await waitFor(() => {
      expect(document.head.querySelector("link[rel='icon']")).toBeInTheDocument();
    });
    expect(document.head.querySelector("link[rel='icon']")).toHaveAttribute("href", "https://cdn.test/favicon.ico");
  });

  it("keeps the upstream fallback while the custom theme fields are null", async () => {
    transport.mockResolvedValue(themeResponse({}));

    renderThemed();

    await waitFor(() => {
      expect(transport).toHaveBeenCalled();
    });
    expect(screen.getByRole("img", { name: "betterlite" })).toHaveAttribute("src", "http://proxy.test:4000/get_image");
  });

  it("starts on the fallback logo before theme settings answer", () => {
    transport.mockReturnValue(new Promise(() => {}));

    renderThemed();

    expect(screen.getByRole("img", { name: "betterlite" })).toHaveAttribute("src", "http://proxy.test:4000/get_image");
    expect(document.head.querySelector("link[rel='icon']")).not.toBeInTheDocument();
  });
});
