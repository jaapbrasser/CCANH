import type { ReactElement } from "react";
import { render } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, useLocation } from "react-router";
import App from "../App";

function client() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
}

export function renderWithQuery(ui: ReactElement) {
  return render(<QueryClientProvider client={client()}>{ui}</QueryClientProvider>);
}

/** Exposes the router's current location for assertions. */
function LocationProbe() {
  const location = useLocation();
  return (
    <div data-testid="location" hidden>
      {location.pathname + location.search}
    </div>
  );
}

/** Render the whole app at `path`. */
export function renderApp(path: string) {
  return render(
    <QueryClientProvider client={client()}>
      <MemoryRouter initialEntries={[path]}>
        <App />
        <LocationProbe />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}
