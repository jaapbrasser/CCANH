import { Navigate, Route, Routes } from "react-router";
import { RequireAdmin, RequireAuth } from "./auth/guards";
import { useUnauthorizedListener } from "./auth/session";
import { Layout } from "./components/Layout";
import { AccountPage } from "./pages/AccountPage";
import { AuditPage } from "./pages/AuditPage";
import { LoginPage } from "./pages/LoginPage";
import { SetupPage } from "./pages/SetupPage";
import { UsersPage } from "./pages/UsersPage";
import { CredentialSetsPage } from "./pages/CredentialSetsPage";
import { ActivityPage } from "./pages/ActivityPage";
import { SettingsPage } from "./pages/SettingsPage";
import { DashboardsPage } from "./pages/dashboards/DashboardsPage";
import { OrganizationPage } from "./pages/OrganizationPage";
import { StreamsPage } from "./pages/StreamsPage";
import { CameraDetailPage } from "./pages/cameras/CameraDetailPage";
import { CamerasPage } from "./pages/cameras/CamerasPage";

export default function App() {
  useUnauthorizedListener();
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route path="/setup" element={<SetupPage />} />
      <Route
        element={
          <RequireAuth>
            <Layout />
          </RequireAuth>
        }
      >
        <Route index element={<Navigate to="/cameras" replace />} />
        <Route path="cameras" element={<CamerasPage />} />
        <Route path="cameras/:id" element={<CameraDetailPage />} />
        <Route path="dashboards" element={<DashboardsPage />} />
        <Route path="dashboards/:id" element={<DashboardsPage />} />
        <Route path="favorites" element={<CamerasPage key="favorites" favoritesOnly />} />
        <Route
          path="organization"
          element={
            <RequireAdmin>
              <OrganizationPage />
            </RequireAdmin>
          }
        />
        <Route path="account" element={<AccountPage />} />
        <Route path="activity" element={<ActivityPage />} />
        <Route
          path="streams"
          element={
            <RequireAdmin>
              <StreamsPage />
            </RequireAdmin>
          }
        />
        <Route
          path="settings"
          element={
            <RequireAdmin>
              <SettingsPage />
            </RequireAdmin>
          }
        />
        <Route
          path="credentials"
          element={
            <RequireAdmin>
              <CredentialSetsPage />
            </RequireAdmin>
          }
        />
        <Route
          path="users"
          element={
            <RequireAdmin>
              <UsersPage />
            </RequireAdmin>
          }
        />
        <Route
          path="audit"
          element={
            <RequireAdmin>
              <AuditPage />
            </RequireAdmin>
          }
        />
        <Route path="*" element={<NotFound />} />
      </Route>
    </Routes>
  );
}

function NotFound() {
  return (
    <div>
      <h1 className="text-xl font-semibold">Page not found</h1>
    </div>
  );
}
