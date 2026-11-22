# ADR 0012: Navigation and settings area

- Status: accepted
- Date: 2026-11-19
- Phase: 11 (site feedback, issues #23–#26)

## Context

By 0.2.0 the top bar held up to eleven links (Dashboards, Cameras,
Favorites, Activity and seven admin pages) plus Sign out. Site feedback asked
for a calmer top bar: the day-to-day pages up front, administration behind a
settings cog, and sign-out tucked under the user's name.

## Decisions

- **Top bar:** Dashboards and Cameras, then the live indicator, health, the
  user menu and a ⚙ Settings cog at the far right.
- **Settings area** (`/settings/*`, `SettingsLayout`): a sidebar with General
  (the former Settings page, `/settings/general`), Activity, Credentials,
  Users, Groups (`/settings/groups`; "Groups & tags" at `/settings/groups-tags` until tags were removed in 0.4.0), Streams, Audit log and an
  API docs link. Everyone gets the cog; the sidebar lists only pages the role
  may open (viewers: Activity), and `/settings` opens the first of them.
  Admin pages keep `RequireAdmin`, so a typed address still says "Admins only".
- **User menu:** the name opens a menu with Account and Sign out, on hover
  (with a short close delay so the pointer can reach it) and on click, so it
  also works with touch and the keyboard; Esc or a click outside closes it.
- **Favorites:** no separate page. Selecting cameras on Cameras offers Add to /
  Remove from favorites to every user (viewers get only these two actions);
  the existing Favorites only filter shows them. The bulk action calls the
  idempotent per-camera endpoints a few at a time (a page holds at most 50
  cameras), so no new API was needed.
- **Discovery** moves to `/cameras/discovery`, opened by a Discover cameras
  button on Cameras (admins). The static segment outranks `/cameras/:id`.
- **Old addresses** (`/activity`, `/credentials`, `/users`, `/organization`,
  `/streams`, `/audit`, `/discovery`, `/favorites`) forward with `replace`,
  keeping bookmarks and the login `next=` links working; `/favorites` becomes
  `/cameras?favorites=1`.

## Consequences

- Admin pages are one click further away; the sidebar keeps them together.
- New admin pages go into `SETTINGS_PAGES` (`components/SettingsLayout.tsx`)
  and a route under `settings`, not into the top bar.
