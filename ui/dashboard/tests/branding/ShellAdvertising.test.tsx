import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen } from "../test-utils";
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

vi.mock("@/components/LicenseExpiryBanner", () => ({
  LicenseExpiryBanner: () => <div>license expiry advertising</div>,
}));

vi.mock("@/components/liteadmin/LiteAdmin", () => ({
  default: () => <div>assistant launcher</div>,
}));

vi.mock("@/lib/http/client", () => ({
  createApiClient: () => ({ get: vi.fn(), post: vi.fn() }),
}));

const transport = vi.fn<typeof fetch>();

describe("dashboard shell advertising", () => {
  beforeEach(() => {
    transport.mockReset();
    vi.stubGlobal("fetch", transport);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("keeps the operational warnings, software update banner and assistant", () => {
    render(
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

  it("mounts no license-expiry advertising", () => {
    render(
      <Layout>
        <div>dashboard page</div>
      </Layout>,
    );

    expect(screen.queryByText("license expiry advertising")).not.toBeInTheDocument();
    expect(screen.queryByText(/Premium/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Upgrade to Premium/)).not.toBeInTheDocument();
  });

  it("fetches no license or seat usage for the shell", () => {
    render(
      <Layout>
        <div>dashboard page</div>
      </Layout>,
    );

    const urls = transport.mock.calls.map(([input]) => String(input));
    expect(urls.filter((url) => url.includes("/license") || url.includes("remaining"))).toEqual([]);
  });
});
