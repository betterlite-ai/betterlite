import { describe, expect, it } from "vitest";
import { render, screen } from "../test-utils";
import { BrandWordmark } from "@/components/branding/BrandWordmark";

describe("BrandWordmark", () => {
  it("renders the full product name as text", () => {
    render(<BrandWordmark />);

    expect(screen.getByText("betterlite")).toHaveAttribute("data-brand-wordmark", "betterlite");
  });

  it("renders a single letter when collapsed", () => {
    render(<BrandWordmark collapsed />);

    expect(screen.getByText("b")).toHaveAttribute("data-brand-wordmark", "betterlite");
    expect(screen.queryByText("betterlite")).not.toBeInTheDocument();
  });

  it("uses the system sans stack and the theme colors instead of an image", () => {
    render(<BrandWordmark />);

    const mark = screen.getByText("betterlite");
    expect(mark).toHaveClass("font-sans", "text-foreground", "dark:text-sidebar-foreground");
    expect(mark).not.toHaveClass("font-serif", "font-mono");
  });

  it("keeps caller classes so a shell can size the mark in place", () => {
    render(<BrandWordmark className="h-5" />);

    expect(screen.getByText("betterlite")).toHaveClass("h-5", "font-sans");
  });
});
