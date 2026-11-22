import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { renderApp } from "../test/render";
import { ADMIN, VIEWER, problem, server, signedInAs } from "../test/server";

describe("login", () => {
  it("redirects signed-out users to login, then back to where they were going", async () => {
    signedInAs(null);
    let posted: unknown;
    server.use(
      http.post("/api/v1/auth/login", async ({ request }) => {
        posted = await request.json();
        signedInAs(ADMIN);
        return HttpResponse.json(ADMIN);
      }),
      http.get("/api/v1/users", () => HttpResponse.json({ items: [], total: 0, page: 1, page_size: 25 })),
    );
    renderApp("/users");

    await screen.findByRole("heading", { name: "Sign in" });
    expect(screen.getByTestId("location")).toHaveTextContent("/login?next=%2Fusers");

    await userEvent.type(screen.getByLabelText("Username"), "admin");
    await userEvent.type(screen.getByLabelText("Password"), "correct horse battery");
    await userEvent.click(screen.getByRole("button", { name: "Sign in" }));

    await screen.findByRole("heading", { name: "Users" });
    expect(posted).toEqual({ username: "admin", password: "correct horse battery" });
    // The old address forwards to its place in the settings area.
    expect(screen.getByTestId("location")).toHaveTextContent(/^\/settings\/users$/);
  });

  it("shows the server's error and clears the password on failure", async () => {
    signedInAs(null);
    server.use(
      http.post("/api/v1/auth/login", () =>
        problem(401, "invalid_credentials", "The username or password is incorrect."),
      ),
    );
    renderApp("/login");
    await userEvent.type(await screen.findByLabelText("Username"), "admin");
    await userEvent.type(screen.getByLabelText("Password"), "wrong password");
    await userEvent.click(screen.getByRole("button", { name: "Sign in" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("The username or password is incorrect.");
    expect(screen.getByLabelText("Password")).toHaveValue("");
  });

  it("explains rate limiting with the retry delay", async () => {
    signedInAs(null);
    server.use(
      http.post("/api/v1/auth/login", () =>
        problem(429, "rate_limited", "Too many attempts.", { "retry-after": "120" }),
      ),
    );
    renderApp("/login");
    await userEvent.type(await screen.findByLabelText("Username"), "admin");
    await userEvent.type(screen.getByLabelText("Password"), "whatever pw");
    await userEvent.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Try again in 2 minutes.");
  });

  it("sends users to setup when no accounts exist", async () => {
    signedInAs(null);
    server.use(http.get("/api/v1/setup", () => HttpResponse.json({ setup_required: true })));
    renderApp("/login");
    await screen.findByRole("heading", { name: "First-run setup" });
  });

  it("ignores off-site next parameters", async () => {
    signedInAs(VIEWER);
    renderApp("/login?next=//evil.example");
    await screen.findByRole("heading", { name: "Cameras" });
    expect(screen.getByTestId("location")).toHaveTextContent(/^\/cameras$/);
  });
});

describe("authorization in the UI", () => {
  it("hides admin navigation from viewers and blocks admin pages", async () => {
    signedInAs(VIEWER);
    renderApp("/users");
    await screen.findByRole("heading", { name: "Admins only" });
    const main = screen.getByRole("navigation", { name: "Main" });
    expect(main).toHaveTextContent("Dashboards");
    expect(main).not.toHaveTextContent("Discovery");
    expect(main).not.toHaveTextContent("Favorites");
    // Viewers get the cog, with only the pages they may open.
    const settings = screen.getByRole("navigation", { name: "Settings" });
    expect(
      within(settings)
        .getAllByRole("link")
        .map((l) => l.textContent),
    ).toEqual(["Activity"]);
  });

  it("puts the admin pages in the settings sidebar", async () => {
    signedInAs(ADMIN);
    renderApp("/");
    await userEvent.click(await screen.findByRole("link", { name: "Settings" }));
    const settings = await screen.findByRole("navigation", { name: "Settings" });
    expect(
      within(settings)
        .getAllByRole("link")
        .map((l) => l.textContent),
    ).toEqual(["General", "Activity", "Credentials", "Users", "Groups", "Streams", "Audit log", "API docs ↗"]);
    expect(screen.getByTestId("location")).toHaveTextContent(/^\/settings\/general$/);
    expect(await screen.findByRole("heading", { name: "General" })).toBeInTheDocument();
    await userEvent.click(within(settings).getByRole("link", { name: "Audit log" }));
    expect(await screen.findByRole("heading", { name: "Audit log" })).toBeInTheDocument();
  });

  it("sends viewers' /settings to Activity", async () => {
    signedInAs(VIEWER);
    renderApp("/settings");
    await waitFor(() => expect(screen.getByTestId("location")).toHaveTextContent(/^\/settings\/activity$/));
  });

  it("returns to the login page when the session expires mid-use", async () => {
    signedInAs(ADMIN);
    server.use(
      http.get("/api/v1/users", () => {
        signedInAs(null);
        return problem(401, "unauthorized", "Sign in to continue.");
      }),
    );
    renderApp("/users");
    await screen.findByRole("heading", { name: "Sign in" });
  });

  it("signs out", async () => {
    signedInAs(VIEWER);
    server.use(
      http.post("/api/v1/auth/logout", () => {
        signedInAs(null);
        return new HttpResponse(null, { status: 204 });
      }),
    );
    renderApp("/");
    expect(screen.queryByRole("menuitem", { name: "Sign out" })).not.toBeInTheDocument();
    // Hovering the name opens the menu; clicking toggles it too.
    await userEvent.hover(await screen.findByRole("button", { name: /^viewer/ }));
    const menu = await screen.findByRole("menu", { name: "Account menu" });
    expect(within(menu).getByRole("menuitem", { name: "Account" })).toHaveAttribute("href", "/account");
    await userEvent.click(within(menu).getByRole("menuitem", { name: "Sign out" }));
    await screen.findByRole("heading", { name: "Sign in" });
  });
});
