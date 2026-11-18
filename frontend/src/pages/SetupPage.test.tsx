import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { renderApp } from "../test/render";
import { ADMIN, problem, server, signedInAs } from "../test/server";

async function fill(token: string, password: string, confirm = password) {
  await userEvent.type(await screen.findByLabelText("Setup token"), token);
  await userEvent.type(screen.getByLabelText("Password"), password);
  await userEvent.type(screen.getByLabelText("Confirm password"), confirm);
  await userEvent.click(screen.getByRole("button", { name: "Create admin" }));
}

describe("setup", () => {
  beforeEach(() => {
    signedInAs(null);
    server.use(http.get("/api/v1/setup", () => HttpResponse.json({ setup_required: true })));
  });

  it("checks the password locally before submitting", async () => {
    renderApp("/setup");
    await fill("tok", "long enough password", "different password");
    expect(await screen.findByRole("alert")).toHaveTextContent("Passwords don't match.");
  });

  it("shows a wrong-token error", async () => {
    server.use(http.post("/api/v1/setup", () => problem(401, "unauthorized", "Sign in to continue.")));
    renderApp("/setup");
    await fill("bad-token", "long enough password");
    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });

  it("creates the admin and signs in", async () => {
    let body: Record<string, string> = {};
    server.use(
      http.post("/api/v1/setup", async ({ request }) => {
        body = (await request.json()) as Record<string, string>;
        signedInAs(ADMIN);
        server.use(http.get("/api/v1/setup", () => HttpResponse.json({ setup_required: false })));
        return HttpResponse.json(ADMIN, { status: 201 });
      }),
    );
    renderApp("/setup");
    await fill("  the-token  ", "long enough password");
    await screen.findByRole("heading", { name: "Cameras" });
    expect(body).toEqual({ token: "the-token", username: "admin", password: "long enough password" });
  });

  it("redirects to login once setup is done", async () => {
    server.use(http.get("/api/v1/setup", () => HttpResponse.json({ setup_required: false })));
    renderApp("/setup");
    await screen.findByRole("heading", { name: "Sign in" });
  });
});
