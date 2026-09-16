// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { WebLoginScreen } from "../../src/components/WebLoginScreen";
import { WebLibraryPicker } from "../../src/components/WebLibraryPicker";

afterEach(cleanup);

describe("WebLoginScreen", () => {
  it("submits the password and surfaces authentication failures", async () => {
    const onLogin = vi
      .fn<(password: string) => Promise<void>>()
      .mockRejectedValue(new Error("invalid credentials"));

    render(<WebLoginScreen onLogin={onLogin} />);
    fireEvent.change(screen.getByLabelText("Password"), {
      target: { value: "secret" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));

    await waitFor(() => expect(onLogin).toHaveBeenCalledWith("secret"));
    expect(await screen.findByText("invalid credentials")).toBeTruthy();
  });

  it("disables the form while a login request is pending", async () => {
    let resolveLogin!: () => void;
    const onLogin = vi.fn(
      () => new Promise<void>((resolve) => {
        resolveLogin = resolve;
      }),
    );

    render(<WebLoginScreen onLogin={onLogin} />);
    fireEvent.change(screen.getByLabelText("Password"), {
      target: { value: "secret" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));

    await waitFor(() => {
      const button = screen.getByRole("button", { name: "Signing in…" }) as HTMLButtonElement;
      expect(button.disabled).toBe(true);
    });
    resolveLogin();
  });
});

describe("WebLibraryPicker", () => {
  it("lists mounted roots and selects the requested root", () => {
    const onSelect = vi.fn();
    render(
      <WebLibraryPicker
        roots={[
          { id: "jazz", name: "jazz", path: "/libraries/jazz" },
          { id: "classical", name: "classical", path: "/libraries/classical" },
        ]}
        onSelect={onSelect}
      />,
    );

    expect(screen.getByText("Choose a library")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /jazz/ }));
    expect(onSelect).toHaveBeenCalledWith("/libraries/jazz");
  });
});
