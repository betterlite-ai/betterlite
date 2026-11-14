import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders, screen, waitFor } from "../test-utils";
import LiteAdmin from "@/components/liteadmin/LiteAdmin";

const auth = vi.hoisted(() => ({
  current: {
    userId: "test-user-id",
    userRole: "Admin",
    isViewOnly: false,
    isAuthorized: true,
    isLoading: false,
    token: "test-token",
    accessToken: "test-access-token",
  },
}));

vi.mock("@/app/(dashboard)/hooks/useAuthorized", () => ({
  default: () => auth.current,
}));

vi.mock("@/app/(dashboard)/hooks/proxySettings/useProxySettings", () => ({
  useProxySettingsQuery: () => ({
    data: undefined,
    isPending: true,
    refetch: vi.fn(),
  }),
}));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn() }),
}));

describe("betterlite Assistant", () => {
  beforeEach(() => {
    auth.current = {
      userId: "test-user-id",
      userRole: "Admin",
      isViewOnly: false,
      isAuthorized: true,
      isLoading: false,
      token: "test-token",
      accessToken: "test-access-token",
    };
  });

  it("labels the launcher and its panel betterlite Assistant", async () => {
    const user = userEvent.setup();
    renderWithProviders(<LiteAdmin />);

    const launcher = screen.getByRole("button", {
      name: "betterlite Assistant",
    });
    await user.click(launcher);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Close betterlite Assistant" })).toBeInTheDocument();
    });
    expect(screen.getByText("Ask about your gateway. Review changes in chat.")).toBeInTheDocument();
    expect(screen.queryByText(/LiteAdmin/)).not.toBeInTheDocument();
  });

  it("stays hidden for a user without the proxy admin role", () => {
    auth.current = { ...auth.current, userRole: "Internal User" };

    renderWithProviders(<LiteAdmin />);

    expect(screen.queryByRole("button", { name: "betterlite Assistant" })).not.toBeInTheDocument();
    expect(screen.queryByText("betterlite Assistant")).not.toBeInTheDocument();
  });

  it("stays hidden for a view-only admin", () => {
    auth.current = { ...auth.current, isViewOnly: true };

    renderWithProviders(<LiteAdmin />);

    expect(screen.queryByRole("button", { name: "betterlite Assistant" })).not.toBeInTheDocument();
  });
});
