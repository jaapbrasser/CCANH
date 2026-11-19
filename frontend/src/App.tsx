import { Navigate, Route, Routes, useLocation } from "react-router";
import { RequireAdmin, RequireAuth } from "./auth/guards";
import { useUnauthorizedListener } from "./auth/session";
import { Layout } from "./components/Layout";
import { SettingsIndex, SettingsLayout } from "./components/SettingsLayout";
import { usePageTitle } from "./components/pageTitle";
import { AccountPage } from "./pages/AccountPage";
import { AuditPage } from "./pages/AuditPage";
import { LoginPage } from "./pages/LoginPage";
import { SetupPage } from "./pages/SetupPage";
import { UsersPage } from "./pages/UsersPage";
import { CredentialSetsPage } from "./pages/CredentialSetsPage";
import { ActivityPage } from "./pages/ActivityPage";
import { SettingsPage } from "./pages/SettingsPage";
import { DashboardsPage } from "./pages/dashboards/DashboardsPage";
import { DiscoveryPage } from "./pages/DiscoveryPage";
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
        <Route
          path="cameras/discovery"
          element={
            <RequireAdmin>
              <DiscoveryPage />
            </RequireAdmin>
          }
        />
        <Route path="account" element={<AccountPage />} />
        <Route path="settings" element={<SettingsLayout />}>
          <Route index element={<SettingsIndex />} />
          <Route
            path="general"
            element={
              <RequireAdmin>
                <SettingsPage />
              </RequireAdmin>
            }
          />
          <Route path="activity" element={<ActivityPage />} />
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
            path="groups-tags"
            element={
              <RequireAdmin>
                <OrganizationPage />
              </RequireAdmin>
            }
          />
          <Route
            path="streams"
            element={
              <RequireAdmin>
                <StreamsPage />
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
        </Route>
        {/* Addresses from before 0.3.0. */}
        <Route path="favorites" element={<Navigate to="/cameras?favorites=1" replace />} />
        <Route path="discovery" element={<Navigate to="/cameras/discovery" replace />} />
        <Route path="activity" element={<Navigate to="/settings/activity" replace />} />
        <Route path="credentials" element={<Navigate to="/settings/credentials" replace />} />
        <Route path="users" element={<KeepQuery to="/settings/users" />} />
        <Route path="organization" element={<Navigate to="/settings/groups-tags" replace />} />
        <Route path="streams" element={<Navigate to="/settings/streams" replace />} />
        <Route path="audit" element={<KeepQuery to="/settings/audit" />} />
        <Route path="*" element={<NotFound />} />
      </Route>
    </Routes>
  );
}

/** Forward an old address, keeping its query string (e.g. page numbers). */
function KeepQuery({ to }: { to: string }) {
  const { search } = useLocation();
  return <Navigate to={to + search} replace />;
}

function NotFound() {
  usePageTitle("Page not found");
  return (
    <div>
      <h1 className="text-xl font-semibold">Page not found</h1>
    </div>
  );
}
