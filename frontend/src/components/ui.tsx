import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode } from "react";
import { useId } from "react";

export function Button({
  variant = "primary",
  className = "",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "primary" | "secondary" | "danger" }) {
  const styles = {
    primary: "bg-amber-500 text-slate-950 hover:bg-amber-400",
    secondary: "border border-slate-700 text-slate-200 hover:bg-slate-800",
    danger: "bg-red-600 text-white hover:bg-red-500",
  }[variant];
  return (
    <button
      type="button"
      className={`rounded px-3 py-1.5 text-sm font-medium disabled:cursor-not-allowed disabled:opacity-50 ${styles} ${className}`}
      {...props}
    />
  );
}

export function Field({
  label,
  hint,
  ...props
}: InputHTMLAttributes<HTMLInputElement> & { label: string; hint?: string }) {
  const id = useId();
  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="text-sm text-slate-300">
        {label}
      </label>
      <input
        id={id}
        className="rounded border border-slate-700 bg-slate-900 px-3 py-2 text-slate-100 outline-none focus:border-amber-500"
        {...props}
      />
      {hint && <p className="text-xs text-slate-500">{hint}</p>}
    </div>
  );
}

export function Alert({ kind = "error", children }: { kind?: "error" | "success"; children: ReactNode }) {
  const styles =
    kind === "error"
      ? "border-red-800 bg-red-950/60 text-red-200"
      : "border-emerald-800 bg-emerald-950/60 text-emerald-200";
  return (
    <div role={kind === "error" ? "alert" : "status"} className={`rounded border px-3 py-2 text-sm ${styles}`}>
      {children}
    </div>
  );
}

export function Pagination({
  page,
  pageSize,
  total,
  onPage,
}: {
  page: number;
  pageSize: number;
  total: number;
  onPage: (page: number) => void;
}) {
  const pages = Math.max(1, Math.ceil(total / pageSize));
  return (
    <nav aria-label="Pagination" className="flex items-center justify-between text-sm text-slate-400">
      <span>
        {total} result{total === 1 ? "" : "s"}
      </span>
      <div className="flex items-center gap-2">
        <Button variant="secondary" disabled={page <= 1} onClick={() => onPage(page - 1)}>
          Previous
        </Button>
        <span>
          Page {page} of {pages}
        </span>
        <Button variant="secondary" disabled={page >= pages} onClick={() => onPage(page + 1)}>
          Next
        </Button>
      </div>
    </nav>
  );
}

/** A minimal modal. Escape or the backdrop cancels. */
const DIALOG_WIDTH = { md: "max-w-md", lg: "max-w-2xl", xl: "max-w-4xl" } as const;

export function Dialog({
  title,
  children,
  onClose,
  size = "md",
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  /** Width: `md` for confirmations, `lg`/`xl` for long forms (issue #28). */
  size?: keyof typeof DIALOG_WIDTH;
}) {
  const id = useId();
  return (
    <div
      className="fixed inset-0 z-10 flex items-center justify-center bg-black/60 p-4"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
      onKeyDown={(e) => e.key === "Escape" && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={id}
        className={`w-full ${DIALOG_WIDTH[size]} rounded-lg border border-slate-700 bg-slate-900 p-5 shadow-xl`}
      >
        <h2 id={id} className="mb-4 text-lg font-semibold">
          {title}
        </h2>
        {children}
      </div>
    </div>
  );
}

/**
 * The buttons at the bottom of a long dialog form: they stay in view while the
 * form scrolls (the form must be the scroll container) and wrap when narrow.
 */
export function DialogFooter({ children, between = false }: { children: ReactNode; between?: boolean }) {
  return (
    <div
      className={`sticky bottom-0 flex flex-wrap items-center gap-2 border-t border-slate-800 bg-slate-900 pt-3 ${
        between ? "justify-between" : "justify-end"
      }`}
    >
      {children}
    </div>
  );
}

export function PageHeader({ title, actions }: { title: string; actions?: ReactNode }) {
  return (
    <div className="mb-6 flex items-center justify-between gap-4">
      <h1 className="text-2xl font-semibold">{title}</h1>
      {actions}
    </div>
  );
}
