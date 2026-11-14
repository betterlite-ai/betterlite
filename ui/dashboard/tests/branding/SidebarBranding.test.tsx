import { ThemeProvider as NextThemesProvider } from "next-themes";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders, screen, waitFor, within } from "../test-utils";
import Sidebar from "@/components/leftnav";
import { BETTERLITE_RELEASES_URL } from "@/components/branding/brandLinks";

const auth = vi.hoisted(() => ({
  current: {
    userId: "test-user-id",
    accessToken: "test-access-token",
    userRole: "Admin",
    userRoleLabel: "Admin",
    isViewOnly: false,
    token: "test-token",
    userEmail: "test@example.com",
    premiumUser: false,
    disabledPersonalKeyCreation: false,
    showSSOBanner: false,
  },
}));

vi.mock("next/navigation", () => ({
  usePathname: () => "/ui/api-keys",
  useRouter: () => ({ push: vi.fn() }),
}));

vi.mock("@/app/(dashboard)/hooks/useAuthorized", () => ({
  default: () => auth.current,
}));

vi.mock("@/app/(dashboard)/hooks/organizations/useOrganizations", () => ({
  useOrganizations: () => ({ data: [], isLoading: false, error: null }),
}));

vi.mock("@/app/(dashboard)/hooks/teams/useTeams", () => ({
  useTeams: () => ({ data: [], isLoading: false, error: null }),
}));

vi.mock("@/app/(dashboard)/hooks/uiConfig/useUIConfig", () => ({
  useUIConfig: () => ({ data: { admin_ui_disabled: false }, isLoading: false }),
}));

vi.mock("@/app/(dashboard)/hooks/healthReadiness/useHealthReadinessDetails", () => ({
  useHealthReadinessDetails: () => ({ data: { litellm_version: "1.105.1" } }),
}));

vi.mock("@/app/(dashboard)/hooks/useLogout", () => ({
  useLogout: () => vi.fn(),
  revokeSessionAndClearClientState: vi.fn(),
}));

const theme = vi.hoisted(() => ({
  logoUrl: null as string | null,
  logoUrlDark: null as string | null,
}));

vi.mock("@/contexts/ThemeContext", () => ({
  useTheme: () => ({
    logoUrl: theme.logoUrl,
    logoUrlDark: theme.logoUrlDark,
    faviconUrl: null,
    setLogoUrl: vi.fn(),
    setLogoUrlDark: vi.fn(),
    setFaviconUrl: vi.fn(),
  }),
}));

const renderSidebar = () => renderWithProviders(<Sidebar collapsed={false} />);

describe("shared sidebar branding", () => {
  beforeEach(() => {
    theme.logoUrl = null;
    theme.logoUrlDark = null;
    auth.current = {
      userId: "test-user-id",
      accessToken: "test-access-token",
      userRole: "Admin",
      userRoleLabel: "Admin",
      isViewOnly: false,
      token: "test-token",
      userEmail: "test@example.com",
      premiumUser: false,
      disabledPersonalKeyCreation: false,
      showSSOBanner: false,
    };
  });

  it("names the home link betterlite and keeps it on the UI root", () => {
    renderSidebar();

    const home = screen.getByRole("link", { name: "betterlite home" });
    expect(home).toHaveAttribute("href", "/ui/");
    expect(home).toHaveTextContent("betterlite");
    expect(screen.queryByRole("link", { name: /litellm home/i })).not.toBeInTheDocument();
  });

  it("shows the wordmark until the configured logo image loads", () => {
    renderSidebar();

    const home = screen.getByRole("link", { name: "betterlite home" });
    const image = within(home).getByRole("img", { name: "betterlite" });
    expect(image).toHaveAttribute("src", expect.stringMatching(/\/get_image$/));
    expect(image).toHaveClass("hidden");
  });

  it("tries the configured dark logo first under a dark theme", async () => {
    theme.logoUrl = "https://cdn.test/logo.png";
    theme.logoUrlDark = "https://cdn.test/logo-dark.png";

    renderWithProviders(
      <NextThemesProvider attribute="class" defaultTheme="dark">
        <Sidebar collapsed={false} />
      </NextThemesProvider>,
    );

    const home = screen.getByRole("link", { name: "betterlite home" });
    await waitFor(() => {
      expect(within(home).getByRole("img", { name: "betterlite" })).toHaveAttribute(
        "src",
        "https://cdn.test/logo-dark.png",
      );
    });
  });

  it("sends the installed version to betterlite's release history", () => {
    renderSidebar();

    expect(screen.getByRole("link", { name: "v1.105.1" })).toHaveAttribute("href", BETTERLITE_RELEASES_URL);
  });

  it("labels Learning Resources as upstream LiteLLM documentation", () => {
    renderSidebar();

    expect(screen.getByText("Learning Resources")).toBeInTheDocument();
    expect(screen.getByText("Upstream LiteLLM documentation")).toBeInTheDocument();
    expect(screen.getByText("Learning Resources").closest("a")).toHaveAttribute(
      "href",
      "https://models.litellm.ai/cookbook",
    );
  });

  it("mounts no seat or license usage meter in the footer", () => {
    const transport = vi.fn<typeof fetch>();
    vi.stubGlobal("fetch", transport);

    renderSidebar();

    expect(screen.queryByText("Enterprise usage")).not.toBeInTheDocument();
    expect(screen.queryByText("Seats")).not.toBeInTheDocument();
    const licenseCalls = transport.mock.calls
      .map(([input]) => String(input))
      .filter((url) => url.includes("/health/license") || url.includes("/user/available_users"));
    expect(licenseCalls).toEqual([]);
    vi.unstubAllGlobals();
  });

  it("keeps the account menu in the footer", () => {
    renderSidebar();

    expect(screen.getByRole("button", { name: /account menu/i })).toBeInTheDocument();
  });

  it("keeps admin-only entries hidden from an internal user", () => {
    auth.current = {
      ...auth.current,
      userRole: "Internal User",
      userRoleLabel: "Internal User",
    };

    renderSidebar();

    expect(screen.getByText("Virtual Keys")).toBeInTheDocument();
    expect(screen.queryByText("Internal Users")).not.toBeInTheDocument();
    expect(screen.queryByText("Settings")).not.toBeInTheDocument();
  });
});
