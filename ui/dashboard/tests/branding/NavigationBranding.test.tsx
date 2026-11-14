import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders, screen, waitFor, within } from "../test-utils";
import Navbar from "@/components/navbar";
import { DashboardHeader } from "@/components/DashboardHeader";
import { DOCS_URL } from "@/components/Navbar/DocsLink/DocsLink";
import { AUTO_ROUTER_DOCS_URL } from "@/components/Navbar/NotificationsBell/NotificationsBell";
import {
  BETTERLITE_DOCS_URL,
  BETTERLITE_RELEASES_URL,
  BETTERLITE_SOURCE_URL,
  BETTERLITE_SUPPORT_URL,
} from "@/components/branding/brandLinks";

const health = vi.hoisted(() => ({ version: "1.105.1" as string | undefined }));

vi.mock("next/navigation", () => ({
  usePathname: () => "/ui/api-keys",
  useRouter: () => ({ push: vi.fn() }),
}));

vi.mock("@/app/(dashboard)/hooks/healthReadiness/useHealthReadinessDetails", () => ({
  useHealthReadinessDetails: () => ({
    data: health.version === undefined ? undefined : { litellm_version: health.version },
  }),
}));

vi.mock("@/app/(dashboard)/hooks/useDisableBouncingIcon", () => ({
  useDisableBouncingIcon: () => false,
}));

vi.mock("@/app/(dashboard)/hooks/useDisableShowPrompts", () => ({
  useDisableShowPrompts: () => false,
}));

vi.mock("@/app/(dashboard)/hooks/useLogout", () => ({
  useLogout: () => vi.fn(),
  revokeSessionAndClearClientState: vi.fn(),
}));

vi.mock("@/app/(dashboard)/hooks/uiSettings/useUISettings", () => ({
  useUISettings: () => ({ data: undefined, isLoading: false }),
}));

vi.mock("@/hooks/useWorker", () => ({
  useWorker: () => ({
    isControlPlane: false,
    workers: [],
    selectedWorkerId: null,
    selectedWorker: null,
    selectWorker: vi.fn(),
    disconnectFromWorker: vi.fn(),
  }),
}));

vi.mock("@/contexts/ThemeContext", () => ({
  useTheme: () => ({
    logoUrl: null,
    logoUrlDark: null,
    faviconUrl: null,
    setLogoUrl: vi.fn(),
    setLogoUrlDark: vi.fn(),
    setFaviconUrl: vi.fn(),
  }),
}));

vi.mock("@/components/Navbar/UserDropdown/UserDropdown", () => ({
  default: () => <div data-testid="account-menu" />,
}));

const renderNavbar = () =>
  renderWithProviders(<Navbar accessToken="test-token" isPublicPage={false} sidebarCollapsed={false} />);

describe("shared navbar branding", () => {
  beforeEach(() => {
    health.version = "1.105.1";
  });

  it("links the home mark to the UI root as betterlite", () => {
    renderNavbar();

    const home = screen.getByRole("link", { name: "betterlite home" });
    expect(home).toHaveAttribute("href", "/ui/");
    expect(within(home).getByText("betterlite")).toBeInTheDocument();
  });

  it("shows the text wordmark until the configured logo image loads", () => {
    renderNavbar();

    const home = screen.getByRole("link", { name: "betterlite home" });
    const image = within(home).getByRole("img", { name: "betterlite" });
    expect(image).toHaveAttribute("src", expect.stringMatching(/\/get_image$/));
    expect(image).toHaveClass("hidden");
  });

  it("sends the version badge to betterlite's release history", () => {
    renderNavbar();

    expect(screen.getByRole("link", { name: "v1.105.1" })).toHaveAttribute("href", BETTERLITE_RELEASES_URL);
  });

  it("points Docs at betterlite's documentation entry", () => {
    renderNavbar();

    expect(DOCS_URL).toBe(BETTERLITE_DOCS_URL);
    expect(screen.getByRole("link", { name: "Docs" })).toHaveAttribute("href", BETTERLITE_DOCS_URL);
  });

  it("replaces the Slack and upstream GitHub CTAs with named betterlite links", () => {
    renderNavbar();

    expect(screen.getByRole("link", { name: "betterlite support" })).toHaveAttribute("href", BETTERLITE_SUPPORT_URL);
    expect(screen.getByRole("link", { name: "betterlite source on GitHub" })).toHaveAttribute(
      "href",
      BETTERLITE_SOURCE_URL,
    );
    expect(screen.queryByRole("link", { name: "Join Slack" })).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "LiteLLM on GitHub" })).not.toBeInTheDocument();
  });

  it("keeps the navbar free of the upstream blog menu", () => {
    renderNavbar();

    expect(screen.queryByText("Blog")).not.toBeInTheDocument();
    const upstreamLinks = screen
      .getAllByRole("link")
      .map((link) => link.getAttribute("href") ?? "")
      .filter((href) => href.includes("litellm.ai"));
    expect(upstreamLinks).toEqual([]);
  });

  it("rebrands the auto-router notice but keeps its feature documentation", async () => {
    const user = userEvent.setup();
    renderNavbar();

    await user.click(screen.getByRole("button", { name: "Notifications" }));

    await waitFor(() => {
      expect(screen.getByText("betterlite Auto Router")).toBeInTheDocument();
    });
    expect(screen.getByRole("link", { name: "Upstream LiteLLM documentation" })).toHaveAttribute(
      "href",
      AUTO_ROUTER_DOCS_URL,
    );
    expect(screen.queryByText("Read the docs")).not.toBeInTheDocument();
  });
});

describe("dashboard header branding", () => {
  it("keeps the shared betterlite docs link and drops the upstream blog menu", () => {
    renderWithProviders(<DashboardHeader />);

    expect(screen.getByRole("link", { name: "Docs" })).toHaveAttribute("href", BETTERLITE_DOCS_URL);
    expect(screen.getByRole("link", { name: "betterlite support" })).toHaveAttribute("href", BETTERLITE_SUPPORT_URL);
    expect(screen.queryByText("Blog")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Notifications" })).toBeInTheDocument();
  });
});
