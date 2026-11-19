import { useEffect, useRef, useState } from "react";
import { Link, useNavigate } from "react-router";
import { useLogout, useMe } from "../api/auth";

const itemClass =
  "block w-full px-4 py-2 text-left text-sm text-slate-200 hover:bg-slate-800 focus:bg-slate-800 focus:outline-none";

/** The signed-in user's name; hover or click it for Account and Sign out. */
export function UserMenu() {
  const { data: me } = useMe();
  const logout = useLogout();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const closeTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);
  useEffect(() => () => window.clearTimeout(closeTimer.current), []);

  // A short delay lets the pointer travel from the name into the menu.
  const show = () => {
    window.clearTimeout(closeTimer.current);
    setOpen(true);
  };
  const hide = () => {
    closeTimer.current = window.setTimeout(() => setOpen(false), 150);
  };

  return (
    <div ref={ref} className="relative" onMouseEnter={show} onMouseLeave={hide}>
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        className="rounded px-3 py-1.5 text-sm text-slate-200 hover:text-white"
        onClick={() => setOpen((o) => !o)}
      >
        {me?.username}
        <span className="ml-2 text-xs uppercase text-slate-500">{me?.role}</span>
        <span aria-hidden className="ml-1 text-slate-500">
          ▾
        </span>
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Account menu"
          className="absolute right-0 z-30 mt-1 min-w-40 overflow-hidden rounded border border-slate-700 bg-slate-900 py-1 shadow-lg"
        >
          <Link role="menuitem" to="/account" className={itemClass} onClick={() => setOpen(false)}>
            Account
          </Link>
          <button
            role="menuitem"
            type="button"
            className={itemClass}
            onClick={() => logout.mutate(undefined, { onSettled: () => navigate("/login") })}
          >
            Sign out
          </button>
        </div>
      )}
    </div>
  );
}
