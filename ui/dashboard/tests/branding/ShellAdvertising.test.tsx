import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders, screen, testQueryClient, waitFor } from "../test-utils";
import Layout from "@/app/(dashboard)/layout";

vi.mock("next/navigation", () => ({
  usePathname: () => "/ui/logs",
  useRouter: () => ({ replace: vi.fn() }),
  useSearchParams: () => new URLSearchParams(),
}));

vi.mock("@/contexts/AuthContext", () => ({
  useAuth: () => ({
    accessToken: "test-token",
    authLoading: false,
    passwordResetRequired: false,
  }),
}));

vi.mock("@/contexts/PluginModeContext", () => ({
  PluginModeProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
  usePluginMode: () => ({
    mode: "ai-gateway",
    activePlugin: null,
    plugins: [],
    setMode: vi.fn(),
  }),
}));

vi.mock("@/contexts/ThemeContext", () => ({
  ThemeProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock("@/components/DashboardHeader", () => ({
  DashboardHeader: () => <header>dashboard header</header>,
}));

vi.mock("@/app/(dashboard)/components/SidebarProvider", () => ({
  default: () => <aside>dashboard sidebar</aside>,
}));

vi.mock("@/components/DebugWarningBanner", () => ({
  DebugWarningBanner: () => <div>debug warning</div>,
}));

vi.mock("@/components/NoRedisWarningBanner", () => ({
  NoRedisWarningBanner: () => <div>redis warning</div>,
}));

vi.mock("@/components/EnvCredentialLoginWarningBanner", () => ({
  EnvCredentialLoginWarningBanner: () => <div>credential warning</div>,
}));

vi.mock("@/components/UserBanner", () => ({
  UserBanner: () => <div>operator banner</div>,
}));

vi.mock("@/components/UpgradeBanner", () => ({
  UpgradeBanner: () => <div>software update banner</div>,
}));

vi.mock("@/components/liteadmin/LiteAdmin", () => ({
  default: () => <div>assistant launcher</div>,
}));

vi.mock("@/lib/http/client", () => ({
  createApiClient: () => ({ get: vi.fn(), post: vi.fn() }),
}));

const transport = vi.fn<typeof fetch>();

const daysFromToday = (days: number) => new Date(Date.now() + days * 86_400_000).toISOString().slice(0, 10);

const licenseResponse = () =>
  new Response(
    JSON.stringify({
      has_license: true,
      license_type: "enterprise",
      expiration_date: daysFromToday(3),
      allowed_features: [],
      limits: { max_users: null, max_teams: null },
    }),
    { status: 200, headers: { "Content-Type": "application/json" } },
  );

describe("dashboard shell advertising", () => {
  beforeEach(() => {
    testQueryClient.clear();
    transport.mockReset();
    transport.mockImplementation(() => Promise.resolve(licenseResponse()));
    vi.stubGlobal("fetch", transport);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("keeps the operational warnings, software update banner and assistant", () => {
    renderWithProviders(
      <Layout>
        <div>dashboard page</div>
      </Layout>,
    );

    expect(screen.getByText("dashboard header")).toBeInTheDocument();
    expect(screen.getByText("dashboard sidebar")).toBeInTheDocument();
    expect(screen.getByText("debug warning")).toBeInTheDocument();
    expect(screen.getByText("redis warning")).toBeInTheDocument();
    expect(screen.getByText("credential warning")).toBeInTheDocument();
    expect(screen.getByText("operator banner")).toBeInTheDocument();
    expect(screen.getByText("software update banner")).toBeInTheDocument();
    expect(screen.getByText("assistant launcher")).toBeInTheDocument();
    expect(screen.getByText("dashboard page")).toBeInTheDocument();
  });

  it("mounts no license-expiry advertising even when the proxy reports an expiring license", async () => {
    renderWithProviders(
      <Layout>
        <div>dashboard page</div>
      </Layout>,
    );

    await waitFor(() => {
      expect(testQueryClient.isFetching()).toBe(0);
    });
    expect(screen.queryByText(/Enterprise license/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Premium/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Upgrade to Premium/)).not.toBeInTheDocument();
  });

  it("fetches no license or seat usage for the shell", () => {
    renderWithProviders(
      <Layout>
        <div>dashboard page</div>
      </Layout>,
    );

    const urls = transport.mock.calls.map(([input]) => String(input));
    expect(urls.filter((url) => url.includes("/health/license") || url.includes("/user/available_users"))).toEqual([]);
  });
});
