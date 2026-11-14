import { ThemeProvider as NextThemesProvider } from "next-themes";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "../test-utils";
import { BrandLogo } from "@/components/branding/BrandLogo";

const themeState = vi.hoisted(() => ({
  logoUrl: null as string | null,
  logoUrlDark: null as string | null,
}));

vi.mock("@/contexts/ThemeContext", () => ({
  useTheme: () => ({
    logoUrl: themeState.logoUrl,
    logoUrlDark: themeState.logoUrlDark,
    faviconUrl: null,
    setLogoUrl: vi.fn(),
    setLogoUrlDark: vi.fn(),
    setFaviconUrl: vi.fn(),
  }),
}));

vi.mock("@/components/networking", () => ({
  getProxyBaseUrl: () => "http://proxy.test:4000",
}));

const renderLogo = (collapsed = false, dark = false) => {
  const logo = <BrandLogo collapsed={collapsed} />;
  return render(
    dark ? (
      <NextThemesProvider attribute="class" defaultTheme="dark">
        {logo}
      </NextThemesProvider>
    ) : (
      logo
    ),
  );
};

const brandImage = () => screen.getByRole("img", { name: "betterlite" });

describe("BrandLogo", () => {
  beforeEach(() => {
    themeState.logoUrl = null;
    themeState.logoUrlDark = null;
  });

  it("shows the text wordmark and hides the fallback image until it loads", () => {
    renderLogo();

    expect(screen.getByText("betterlite")).toBeInTheDocument();
    expect(brandImage()).toHaveAttribute("src", "http://proxy.test:4000/get_image");
    expect(brandImage()).toHaveClass("hidden");
  });

  it("shows the configured upstream logo as soon as it loads", () => {
    renderLogo();

    fireEvent.load(brandImage());

    expect(screen.queryByText("betterlite")).not.toBeInTheDocument();
    expect(brandImage()).not.toHaveClass("hidden");
  });

  it("falls back to the wordmark and removes a failed image, never a broken image", () => {
    renderLogo();

    fireEvent.error(brandImage());

    expect(screen.getByText("betterlite")).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: "betterlite" })).not.toBeInTheDocument();
  });

  it("requests the dark fallback under a dark theme", async () => {
    renderLogo(false, true);

    await waitFor(() => {
      expect(brandImage()).toHaveAttribute("src", "http://proxy.test:4000/get_image?theme=dark");
    });
  });

  it("requests the monogram variant when the shell is collapsed", () => {
    renderLogo(true);

    expect(screen.getByText("b")).toBeInTheDocument();
    expect(brandImage()).toHaveAttribute("src", "http://proxy.test:4000/get_image?variant=monogram");
  });

  it("prefers a configured light logo once it loads", () => {
    themeState.logoUrl = "https://cdn.test/logo.png";

    renderLogo();

    expect(brandImage()).toHaveAttribute("src", "https://cdn.test/logo.png");
    fireEvent.load(brandImage());
    expect(screen.queryByText("betterlite")).not.toBeInTheDocument();
  });

  it("prefers a configured dark logo under a dark theme", async () => {
    themeState.logoUrl = "https://cdn.test/logo.png";
    themeState.logoUrlDark = "https://cdn.test/logo-dark.png";

    renderLogo(false, true);

    await waitFor(() => {
      expect(brandImage()).toHaveAttribute("src", "https://cdn.test/logo-dark.png");
    });
  });

  it("reuses the light configured logo in a dark theme when no dark one is set", async () => {
    themeState.logoUrl = "https://cdn.test/logo.png";

    renderLogo(false, true);

    await waitFor(() => {
      expect(brandImage()).toHaveAttribute("src", "https://cdn.test/logo.png");
    });
  });

  it("keeps a configured logo when collapsed instead of the monogram variant", () => {
    themeState.logoUrl = "https://cdn.test/logo.png";

    renderLogo(true);

    expect(screen.getByText("b")).toBeInTheDocument();
    expect(brandImage()).toHaveAttribute("src", "https://cdn.test/logo.png");
  });
});
