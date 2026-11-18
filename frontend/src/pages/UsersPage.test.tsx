import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { renderApp } from "../test/render";
import { ADMIN, VIEWER, problem, server, signedInAs, user } from "../test/server";

describe("users admin", () => {
  beforeEach(() => {
    signedInAs(ADMIN);
    server.use(
      http.get("/api/v1/users", ({ request }) => {
        const q = new URL(request.url).searchParams.get("q");
        const items = [user(ADMIN), user(VIEWER)].filter((u) => !q || u.username.includes(q));
        return HttpResponse.json({ items, total: items.length, page: 1, page_size: 25 });
      }),
    );
  });

  /** Rows once the user list has loaded (header row first). */
  async function rows() {
    await screen.findByRole("cell", { name: /^viewer/ });
    return screen.getAllByRole("row");
  }

  it("lists users and marks the current user", async () => {
    renderApp("/users");
    const all = await rows();
    expect(all).toHaveLength(3); // header + 2
    expect(all[1]).toHaveTextContent("admin(you)");
    expect(within(all[1]!).getByRole("button", { name: "Delete" })).toBeDisabled();
    expect(screen.getByText("2 results")).toBeInTheDocument();
  });

  it("searches", async () => {
    renderApp("/users");
    await rows();
    await userEvent.type(screen.getByLabelText("Search users"), "view");
    await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(2));
  });

  it("creates a user", async () => {
    let body: unknown;
    server.use(
      http.post("/api/v1/users", async ({ request }) => {
        body = await request.json();
        return HttpResponse.json(user({ id: "00000000-0000-7000-8000-000000000003", username: "bob", role: "viewer" }), {
          status: 201,
        });
      }),
    );
    renderApp("/users");
    await userEvent.click(await screen.findByRole("button", { name: "New user" }));
    const dialog = screen.getByRole("dialog", { name: "New user" });
    await userEvent.type(within(dialog).getByLabelText("Username"), "bob");
    await userEvent.type(within(dialog).getByLabelText("Initial password"), "bob's long password");
    await userEvent.click(within(dialog).getByRole("button", { name: "Create" }));
    expect(await screen.findByText("Created bob.")).toBeInTheDocument();
    expect(body).toEqual({ username: "bob", password: "bob's long password", role: "viewer" });
  });

  it("shows the server's explanation when a change is refused", async () => {
    server.use(
      http.patch("/api/v1/users/:id", () =>
        problem(409, "conflict", "This would leave no enabled admin. Promote or enable another admin first."),
      ),
    );
    renderApp("/users");
    await userEvent.selectOptions(await screen.findByLabelText("Role for admin"), "viewer");
    expect(await screen.findByRole("alert")).toHaveTextContent("no enabled admin");
  });

  it("asks for confirmation before deleting", async () => {
    let deleted: string | undefined;
    server.use(
      http.delete("/api/v1/users/:id", ({ params }) => {
        deleted = params.id as string;
        return new HttpResponse(null, { status: 204 });
      }),
    );
    renderApp("/users");
    const all = await rows();
    await userEvent.click(within(all[2]!).getByRole("button", { name: "Delete" }));
    const dialog = screen.getByRole("dialog", { name: "Delete viewer?" });
    expect(deleted).toBeUndefined();
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete user" }));
    await waitFor(() => expect(deleted).toBe(VIEWER.id));
  });

  it("resets a password", async () => {
    let body: unknown;
    server.use(
      http.post("/api/v1/users/:id/password", async ({ request }) => {
        body = await request.json();
        return new HttpResponse(null, { status: 204 });
      }),
    );
    renderApp("/users");
    const all = await rows();
    await userEvent.click(within(all[2]!).getByRole("button", { name: "Reset password" }));
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByLabelText("New password"), "a fresh long password");
    await userEvent.click(within(dialog).getByRole("button", { name: "Reset password" }));
    expect(await screen.findByText(/Password reset for viewer/)).toBeInTheDocument();
    expect(body).toEqual({ new_password: "a fresh long password" });
  });
});
