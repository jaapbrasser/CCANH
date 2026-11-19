import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { renderApp } from "../test/render";
import { ADMIN, problem, server, signedInAs } from "../test/server";

const NOW = "2026-11-19T10:00:00Z";
const SET = { id: "00000000-0000-7000-8000-0000000000c1", name: "Site A", camera_count: 3, created_at: NOW, updated_at: NOW };

describe("credential sets page", () => {
  beforeEach(() => signedInAs(ADMIN));

  it("lists sets without secrets and creates one", async () => {
    let body: unknown;
    server.use(
      http.get("/api/v1/credential-sets", () => HttpResponse.json([SET])),
      http.post("/api/v1/credential-sets", async ({ request }) => {
        body = await request.json();
        return HttpResponse.json({ ...SET, id: "00000000-0000-7000-8000-0000000000c2", name: "Site B", camera_count: 0 }, { status: 201 });
      }),
    );
    renderApp("/credentials");
    const row = (await screen.findByText("Site A")).closest("tr")!;
    expect(row).toHaveTextContent("3");

    await userEvent.click(screen.getByRole("button", { name: "New credential set" }));
    const dialog = screen.getByRole("dialog");
    await userEvent.type(within(dialog).getByLabelText("Name"), "Site B");
    await userEvent.type(within(dialog).getByLabelText("Camera username"), "svc");
    await userEvent.type(within(dialog).getByLabelText("Camera password"), "s3cret-pass");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved Site B.")).toBeInTheDocument();
    expect(body).toEqual({ name: "Site B", username: "svc", password: "s3cret-pass" });
  });

  it("renames without replacing credentials when they're left empty", async () => {
    let body: unknown;
    server.use(
      http.get("/api/v1/credential-sets", () => HttpResponse.json([SET])),
      http.patch("/api/v1/credential-sets/:id", async ({ request }) => {
        body = await request.json();
        return HttpResponse.json({ ...SET, name: "Site A (all)" });
      }),
    );
    renderApp("/credentials");
    await userEvent.click(await screen.findByRole("button", { name: "Edit" }));
    const name = within(screen.getByRole("dialog")).getByLabelText("Name");
    await userEvent.clear(name);
    await userEvent.type(name, "Site A (all)");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByText("Saved Site A (all).");
    expect(body).toEqual({ name: "Site A (all)" });
  });

  it("explains why a set in use can't be deleted", async () => {
    server.use(
      http.get("/api/v1/credential-sets", () => HttpResponse.json([SET])),
      http.delete("/api/v1/credential-sets/:id", () =>
        problem(409, "conflict", "This credential set is used by 3 camera(s). Assign them other credentials first."),
      ),
    );
    renderApp("/credentials");
    await userEvent.click(await screen.findByRole("button", { name: "Delete" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("used by 3 camera(s)");
  });
});

const TASK_ID = "00000000-0000-7000-8000-0000000000f1";
function task(status: string, result: unknown) {
  return {
    id: TASK_ID,
    kind: "test_connection",
    created_by: ADMIN.id,
    created_by_username: "admin",
    status,
    progress_current: status === "succeeded" ? 4 : 0,
    progress_total: 4,
    message: status === "succeeded" ? "Done" : null,
    result,
    created_at: NOW,
  };
}

describe("camera form credentials and test connection", () => {
  beforeEach(() => {
    signedInAs(ADMIN);
    server.use(http.get("/api/v1/credential-sets", () => HttpResponse.json([SET])));
  });

  it("tests the connection and pins the presented certificate", async () => {
    let tested: Record<string, unknown> = {};
    let created: Record<string, unknown> = {};
    const fp = "AB:" .repeat(31) + "AB";
    server.use(
      http.post("/api/v1/cameras/test-connection", async ({ request }) => {
        tested = (await request.json()) as Record<string, unknown>;
        return HttpResponse.json(task("queued", null), { status: 202 });
      }),
      http.get("/api/v1/tasks/:id", () =>
        HttpResponse.json(
          task("succeeded", {
            ok: false,
            response_time_ms: 21,
            device: { manufacturer: "AXIS", model: "AXIS M3086-V", firmware_version: "11.11.73" },
            capabilities: [],
            stream_profiles: [],
            tls_fingerprint_sha256: fp,
            steps: [
              { name: "reachability", ok: true, skipped: false, error_category: null, message: null },
              { name: "credentials", ok: false, skipped: false, error_category: "auth_failed", message: "the camera rejected the configured credentials" },
            ],
          }),
        ),
      ),
      http.post("/api/v1/cameras", async ({ request }) => {
        created = (await request.json()) as Record<string, unknown>;
        return problem(400, "bad_request", "stop here");
      }),
    );
    renderApp("/cameras");
    await userEvent.click(await screen.findByRole("button", { name: "Add camera" }));
    const dialog = screen.getByRole("dialog", { name: "Add camera" });
    await userEvent.type(await within(dialog).findByLabelText("Name"), "Gate");
    await userEvent.type(within(dialog).getByLabelText("Host"), "192.0.2.80");
    await userEvent.selectOptions(within(dialog).getByLabelText("Protocol"), "https");
    await userEvent.selectOptions(within(dialog).getByLabelText("Authentication"), "set");
    await userEvent.selectOptions(within(dialog).getByLabelText("Credential set"), SET.id);
    await userEvent.click(within(dialog).getByRole("button", { name: "Test connection" }));

    const result = await within(dialog).findByRole("region", { name: "Connection test result" });
    expect(result).toHaveTextContent("Connection test found problems");
    expect(result).toHaveTextContent("Authentication failed");
    expect(result).toHaveTextContent("AXIS M3086-V");
    expect(tested).toMatchObject({ host: "192.0.2.80", credentials: { type: "set", id: SET.id } });

    await userEvent.click(within(result).getByRole("button", { name: "Pin this certificate" }));
    expect(within(dialog).getByLabelText("Certificate check")).toHaveValue("pinned");
    expect(within(dialog).getByLabelText("SHA-256 fingerprint")).toHaveValue(fp);

    await userEvent.click(within(dialog).getByRole("button", { name: "Add camera" }));
    await waitFor(() =>
      expect(created).toMatchObject({
        tls_mode: "pinned",
        tls_pinned_sha256: fp,
        credentials: { type: "set", id: SET.id },
      }),
    );
  });

  it("only offers Basic-over-HTTP for plain HTTP and sends inline credentials", async () => {
    let created: Record<string, unknown> = {};
    server.use(
      http.post("/api/v1/cameras", async ({ request }) => {
        created = (await request.json()) as Record<string, unknown>;
        return problem(400, "bad_request", "stop here");
      }),
    );
    renderApp("/cameras");
    await userEvent.click(await screen.findByRole("button", { name: "Add camera" }));
    const dialog = screen.getByRole("dialog", { name: "Add camera" });
    // HTTP with Basic allowed is the shipped default (ADR 0011).
    await userEvent.type(await within(dialog).findByLabelText("Name"), "Old cam");
    expect(within(dialog).getByLabelText(/Allow Basic/)).toBeChecked();
    await userEvent.selectOptions(within(dialog).getByLabelText("Protocol"), "https");
    expect(within(dialog).queryByLabelText(/Allow Basic/)).not.toBeInTheDocument();
    await userEvent.selectOptions(within(dialog).getByLabelText("Protocol"), "http");
    await userEvent.type(within(dialog).getByLabelText("Host"), "192.0.2.81");
    await userEvent.selectOptions(within(dialog).getByLabelText("Authentication"), "inline");
    await userEvent.type(within(dialog).getByLabelText("Camera username"), "root");
    await userEvent.type(within(dialog).getByLabelText("Camera password"), "pw-123");
    await userEvent.click(within(dialog).getByRole("button", { name: "Add camera" }));
    await waitFor(() =>
      expect(created).toMatchObject({
        scheme: "http",
        allow_basic_over_http: true,
        credentials: { type: "inline", username: "root", password: "pw-123" },
      }),
    );
  });
});
