import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, renderWithProviders, screen, waitFor } from "../test-utils";
import { useUIConfig } from "@/app/(dashboard)/hooks/uiConfig/useUIConfig";
import LoginPage from "@/app/login/LoginPage";
import { OnboardingFormBody } from "@/app/onboarding/OnboardingFormBody";
import LoadingScreen from "@/components/common_components/LoadingScreen";
import { getCookieFromDocument } from "@/utils/cookieUtils";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
}));

vi.mock("@/app/(dashboard)/hooks/uiConfig/useUIConfig", () => ({
  useUIConfig: vi.fn(),
}));

vi.mock("@/utils/cookieUtils", () => ({
  clearTokenCookies: vi.fn(),
  getCookieFromDocument: vi.fn(),
}));

vi.mock("@/utils/jwtUtils", () => ({
  isJwtExpired: vi.fn(),
}));

vi.mock("@/components/networking", () => ({
  exchangeLoginCode: vi.fn(),
  getProxyBaseUrl: () => "http://proxy.test:4000",
  switchToWorkerUrl: vi.fn(),
}));

vi.mock("@/app/(dashboard)/hooks/login/useLogin", () => ({
  useLogin: () => ({ mutate: vi.fn(), isPending: false, error: null }),
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

const mockConfig = (data: Record<string, unknown>) => {
  (useUIConfig as ReturnType<typeof vi.fn>).mockReturnValue({
    data,
    isLoading: false,
  });
  (getCookieFromDocument as ReturnType<typeof vi.fn>).mockReturnValue(null);
};

const LOGIN_CONFIG = {
  auto_redirect_to_sso: false,
  server_root_path: "/",
  proxy_base_url: null,
  sso_configured: false,
  admin_ui_disabled: false,
  hide_default_credentials_hint: false,
};

describe("login and loading screens", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows the betterlite wordmark on the loading screen", () => {
    render(<LoadingScreen />);

    expect(screen.getByText("betterlite")).toBeInTheDocument();
    expect(screen.queryByText("🚅 LiteLLM")).not.toBeInTheDocument();
  });

  it("shows betterlite on the enabled login screen", async () => {
    mockConfig(LOGIN_CONFIG);

    renderWithProviders(<LoginPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "Login" })).toBeInTheDocument();
    });
    expect(screen.getByRole("heading", { level: 2, name: "betterlite" })).toBeInTheDocument();
    expect(screen.getByText("Access your betterlite Admin UI.")).toBeInTheDocument();
    expect(screen.getByText(/Password is your set betterlite Proxy/)).toBeInTheDocument();
  });

  it("labels the retained upstream credentials guide as LiteLLM's documentation", async () => {
    mockConfig(LOGIN_CONFIG);

    renderWithProviders(<LoginPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "Login" })).toBeInTheDocument();
    });
    expect(screen.getByRole("link", { name: "Upstream LiteLLM documentation" })).toHaveAttribute(
      "href",
      "https://docs.litellm.ai/docs/proxy/ui",
    );
    expect(screen.queryByText("Check the documentation")).not.toBeInTheDocument();
    expect(screen.queryByText(/Access your LiteLLM Admin UI/)).not.toBeInTheDocument();
  });

  it("shows betterlite on the disabled login screen", async () => {
    mockConfig({ ...LOGIN_CONFIG, admin_ui_disabled: true });

    renderWithProviders(<LoginPage />);

    await waitFor(() => {
      expect(screen.getByText("Admin UI Disabled")).toBeInTheDocument();
    });
    expect(screen.getByRole("heading", { level: 2, name: "betterlite" })).toBeInTheDocument();
    expect(screen.queryByText("🚅 LiteLLM")).not.toBeInTheDocument();
  });

  it("names betterlite in the SSO notice", async () => {
    mockConfig({ ...LOGIN_CONFIG, sso_configured: true });

    renderWithProviders(<LoginPage />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "Login" })).toBeInTheDocument();
    });
    expect(screen.getByText(/betterlite no longer automatically redirects/)).toBeInTheDocument();
  });
});

describe("onboarding screens", () => {
  it("shows the wordmark and no paid-tier SSO advert when signing up", () => {
    render(
      <OnboardingFormBody
        variant="signup"
        userEmail="new@example.com"
        isPending={false}
        claimError={null}
        onSubmit={vi.fn()}
      />,
    );

    expect(screen.getByText("betterlite")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Sign Up" })).toBeInTheDocument();
    expect(screen.queryByText(/Enterprise Tier/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Get Free Trial/)).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /free trial/i })).not.toBeInTheDocument();
  });

  it("shows the wordmark when resetting a password", () => {
    render(
      <OnboardingFormBody
        variant="reset_password"
        userEmail="user@example.com"
        isPending={false}
        claimError={null}
        onSubmit={vi.fn()}
      />,
    );

    expect(screen.getByText("betterlite")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Reset Password" })).toBeInTheDocument();
  });
});
