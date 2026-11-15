import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders, screen, waitFor, within } from "../test-utils";
import SidebarAccountMenu from "@/components/SidebarAccountMenu/SidebarAccountMenu";
import UserDropdown from "@/components/Navbar/UserDropdown/UserDropdown";
import { BETTERLITE_RELEASES_URL } from "@/components/branding/brandLinks";

const auth = vi.hoisted(() => ({
  current: {
    userId: "test-user-id",
    userEmail: "test@example.com",
    userRole: "Admin",
    userRoleLabel: "Admin",
    isViewOnly: false,
    accessToken: "test-token",
    loginMethod: null as string | null,
    premiumUser: true,
  },
}));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn() }),
}));

vi.mock("@/app/(dashboard)/hooks/useAuthorized", () => ({
  default: () => auth.current,
}));

vi.mock("@/app/(dashboard)/hooks/healthReadiness/useHealthReadinessDetails", () => ({
  useHealthReadinessDetails: () => ({ data: { litellm_version: "1.99.0" } }),
}));

vi.mock("@/lib/serverRootPath", () => ({
  serverRootPath: "/proxy",
  setServerRootPath: vi.fn(),
}));

const ADMIN_AUTH = {
  userId: "test-user-id",
  userEmail: "test@example.com",
  userRole: "Admin",
  userRoleLabel: "Admin",
  isViewOnly: false,
  accessToken: "test-token",
  loginMethod: null,
  premiumUser: true,
};

describe("sidebar account menu branding", () => {
  beforeEach(() => {
    auth.current = { ...ADMIN_AUTH };
  });

  const openSidebarMenu = async () => {
    const user = userEvent.setup();
    renderWithProviders(<SidebarAccountMenu onLogout={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: /account menu/i }));
    await waitFor(() => {
      expect(screen.getByTestId("sidebar-account-menu-panel")).toBeInTheDocument();
    });
    return within(screen.getByTestId("sidebar-account-menu-panel"));
  };

  it("names betterlite in the menu header and thanks its user", async () => {
    const panel = await openSidebarMenu();

    expect(panel.getByText("betterlite")).toBeInTheDocument();
    expect(panel.getByTitle("Thanks for using betterlite!")).toBeInTheDocument();
  });

  it("sends the version badge to betterlite's release history", async () => {
    const panel = await openSidebarMenu();

    expect(panel.getByRole("link", { name: "v1.99.0" })).toHaveAttribute("href", BETTERLITE_RELEASES_URL);
  });

  it("shows no Standard or Premium tier, even for a premium session", async () => {
    const panel = await openSidebarMenu();

    expect(panel.queryByText("Premium")).not.toBeInTheDocument();
    expect(panel.queryByText("Standard")).not.toBeInTheDocument();
    expect(panel.queryByText("Tier")).not.toBeInTheDocument();
  });

  it("keeps the role visible and links the bundled Berri AI notice", async () => {
    const panel = await openSidebarMenu();

    expect(panel.getByText("Role")).toBeInTheDocument();
    expect(panel.getByText("Admin")).toBeInTheDocument();
    expect(
      panel.getByRole("link", {
        name: "LiteLLM license and notices (Berri AI, MIT)",
      }),
    ).toHaveAttribute("href", "/proxy/ui/notices/LiteLLM-LICENSE.txt");
  });

  it("renames the assistant toggle for proxy admins", async () => {
    const panel = await openSidebarMenu();

    expect(panel.getByRole("switch", { name: "Toggle hide betterlite Assistant" })).toBeInTheDocument();
  });

  it("keeps the assistant toggle hidden from a non-admin without hiding the role", async () => {
    auth.current = {
      ...ADMIN_AUTH,
      userRole: "Internal User",
      userRoleLabel: "Internal User",
    };

    const panel = await openSidebarMenu();

    expect(panel.getByText("Internal User")).toBeInTheDocument();
    expect(panel.queryByRole("switch", { name: /betterlite Assistant/ })).not.toBeInTheDocument();
  });
});

describe("navbar account menu branding", () => {
  beforeEach(() => {
    auth.current = { ...ADMIN_AUTH };
  });

  const openUserMenu = async () => {
    const user = userEvent.setup();
    renderWithProviders(<UserDropdown onLogout={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: /account menu/i }));
    await waitFor(() => {
      expect(screen.getByTestId("user-dropdown-panel")).toBeInTheDocument();
    });
    return within(screen.getByTestId("user-dropdown-panel"));
  };

  it("shows no Standard or Premium tier, even for a premium session", async () => {
    const panel = await openUserMenu();

    expect(panel.queryByText("Premium")).not.toBeInTheDocument();
    expect(panel.queryByText("Standard")).not.toBeInTheDocument();
  });

  it("keeps the role and links the bundled Berri AI notice", async () => {
    const panel = await openUserMenu();

    expect(panel.getByText("Role")).toBeInTheDocument();
    expect(panel.getByText("Admin")).toBeInTheDocument();
    expect(
      panel.getByRole("link", {
        name: "LiteLLM license and notices (Berri AI, MIT)",
      }),
    ).toHaveAttribute("href", "/proxy/ui/notices/LiteLLM-LICENSE.txt");
  });

  it("renames the assistant toggle for proxy admins", async () => {
    const panel = await openUserMenu();

    expect(panel.getByRole("switch", { name: "Toggle hide betterlite Assistant" })).toBeInTheDocument();
  });
});
