import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "../test-utils";
import { UpgradeBanner, UpgradeBannerView } from "@/components/UpgradeBanner";

const releaseState = vi.hoisted(() => ({
  current: null as null | Record<string, unknown>,
}));

vi.mock("@/app/(dashboard)/hooks/healthReadiness/useHealthReadinessDetails", () => ({
  useHealthReadinessDetails: () => ({ data: { litellm_version: "1.0.0" } }),
}));

vi.mock("@/app/(dashboard)/hooks/latestRelease/useLatestReleaseInfo", () => ({
  useLatestReleaseInfo: () => ({ data: releaseState.current }),
}));

const RELEASE_URL = "https://github.com/betterlite-ai/betterlite/releases/tag/v1.0.1";

const release = (overrides: Record<string, unknown> = {}) => ({
  version: "1.0.1",
  release_url: RELEASE_URL,
  new_features: 2,
  bug_fixes: 1,
  other_updates: 0,
  ...overrides,
});

describe("software update banner", () => {
  beforeEach(() => {
    localStorage.clear();
    releaseState.current = release();
  });

  it("uses the release_url returned by /get/latest_release_info as the link target", () => {
    const returnedUrl = "https://example.test/releases/betterlite?channel=stable";

    render(<UpgradeBannerView currentVersion="1.0.0" latestRelease={release({ release_url: returnedUrl })} />);

    expect(screen.getByRole("link", { name: "v1.0.1" })).toHaveAttribute("href", returnedUrl);
  });

  it("describes the release counts and the installed version", () => {
    render(<UpgradeBannerView currentVersion="1.0.0" latestRelease={release()} />);

    expect(screen.getByText(/2 new features, 1 fix, and 0 other updates/)).toBeInTheDocument();
    expect(screen.getByText("Your current version is v1.0.0")).toBeInTheDocument();
  });

  it("renders nothing for the null release payload and no installed version", () => {
    const { unmount } = render(<UpgradeBannerView currentVersion="1.0.0" latestRelease={null} />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    unmount();

    render(<UpgradeBannerView currentVersion={undefined} latestRelease={release()} />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("renders nothing when the installed version is current or newer", () => {
    const { unmount } = render(<UpgradeBannerView currentVersion="1.0.1" latestRelease={release()} />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    unmount();

    render(<UpgradeBannerView currentVersion="1.1.0" latestRelease={release()} />);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("passes the fetched release payload through the mounted banner", async () => {
    render(<UpgradeBanner accessToken="test-token" />);

    await waitFor(() => {
      expect(screen.getByRole("link", { name: "v1.0.1" })).toHaveAttribute("href", RELEASE_URL);
    });
  });

  it("shows no banner when the release endpoint answers null", () => {
    releaseState.current = null;

    render(<UpgradeBanner accessToken="test-token" />);

    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("dismisses the banner for that release only", async () => {
    const user = userEvent.setup();

    render(<UpgradeBanner accessToken="test-token" />);

    await user.click(screen.getByRole("button", { name: "Close" }));

    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(localStorage.getItem("litellm:upgradeBannerDismissed:1.0.1")).toBe("true");
  });
});
