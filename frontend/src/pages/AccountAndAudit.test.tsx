import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { renderApp } from "../test/render";
import { ADMIN, VIEWER, problem, server, signedInAs } from "../test/server";

describe("account", () => {
  it("changes the password", async () => {
    signedInAs(VIEWER);
    let body: unknown;
    server.use(
      http.post("/api/v1/auth/password", async ({ request }) => {
        body = await request.json();
        return new HttpResponse(null, { status: 204 });
      }),
    );
    renderApp("/account");
    await userEvent.type(await screen.findByLabelText("Current password"), "old password here");
    await userEvent.type(screen.getByLabelText("New password"), "new password here");
    await userEvent.type(screen.getByLabelText("Confirm new password"), "new password here");
    await userEvent.click(screen.getByRole("button", { name: "Change password" }));
    expect(await screen.findByText(/Password changed/)).toBeInTheDocument();
    expect(body).toEqual({ current_password: "old password here", new_password: "new password here" });
  });

  it("shows a wrong-current-password error", async () => {
    signedInAs(VIEWER);
    server.use(
      http.post("/api/v1/auth/password", () =>
        problem(401, "invalid_credentials", "The username or password is incorrect."),
      ),
    );
    renderApp("/account");
    await userEvent.type(await screen.findByLabelText("Current password"), "wrong");
    await userEvent.type(screen.getByLabelText("New password"), "new password here");
    await userEvent.type(screen.getByLabelText("Confirm new password"), "new password here");
    await userEvent.click(screen.getByRole("button", { name: "Change password" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("incorrect");
  });
});

describe("audit log", () => {
  it("lists entries and filters by outcome", async () => {
    signedInAs(ADMIN);
    const seen: string[] = [];
    server.use(
      http.get("/api/v1/audit", ({ request }) => {
        seen.push(new URL(request.url).search);
        return HttpResponse.json({
          items: [
            {
              id: 2,
              occurred_at: "2026-11-18T12:00:00Z",
              actor_username: "mallory",
              action: "auth.login",
              outcome: "failure",
              source_ip: "10.0.0.5",
              details: { reason: "unknown_user" },
            },
          ],
          total: 1,
          page: 1,
          page_size: 50,
        });
      }),
    );
    renderApp("/audit");
    expect(await screen.findByText("mallory")).toBeInTheDocument();
    expect(screen.getByText(/unknown_user/)).toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText("Filter by outcome"), "failure");
    await screen.findByText("mallory");
    expect(seen.at(-1)).toContain("outcome=failure");
  });
});
