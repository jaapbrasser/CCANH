import { useState, type ReactNode } from "react";
import { groupTree, useFavorites, useToggleFavorite, type Group } from "../api/organization";

const selectClass = "rounded border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-100";

/** A single-group picker showing the tree with indentation. */
export function GroupSelect({
  label,
  groups,
  value,
  onChange,
  emptyLabel,
  exclude,
}: {
  label: string;
  groups: Group[];
  value: string;
  onChange: (id: string) => void;
  /** Label of the "" option. */
  emptyLabel: string;
  /** Group ids that can't be picked (e.g. a group's own subtree when moving it). */
  exclude?: Set<string>;
}) {
  return (
    <select aria-label={label} className={selectClass} value={value} onChange={(e) => onChange(e.target.value)}>
      <option value="">{emptyLabel}</option>
      {groupTree(groups)
        .filter((g) => !exclude?.has(g.id))
        .map((g) => (
          <option key={g.id} value={g.id}>
            {"  ".repeat(g.depth)}
            {g.name}
          </option>
        ))}
    </select>
  );
}

export function Chips({ items, label }: { items: { id: string; name: string }[]; label: string }) {
  if (items.length === 0) return <span className="text-slate-600">—</span>;
  return (
    <ul aria-label={label} className="flex flex-wrap gap-1">
      {items.map((i) => (
        <li key={i.id} className="rounded bg-slate-800 px-1.5 py-0.5 text-xs text-slate-200">
          {i.name}
        </li>
      ))}
    </ul>
  );
}

/** Star toggle for the signed-in user's favorites. */
export function FavoriteButton({ cameraId, name }: { cameraId: string; name: string }) {
  const favorites = useFavorites();
  const toggle = useToggleFavorite();
  const on = favorites.data?.includes(cameraId) ?? false;
  return (
    <button
      type="button"
      aria-pressed={on}
      aria-label={on ? `Remove ${name} from favorites` : `Add ${name} to favorites`}
      title={on ? "Remove from favorites" : "Add to favorites"}
      className={`text-lg leading-none ${on ? "text-amber-300" : "text-slate-600 hover:text-slate-300"}`}
      onClick={(e) => {
        e.stopPropagation();
        toggle.mutate({ cameraId, favorite: !on });
      }}
    >
      {on ? "★" : "☆"}
    </button>
  );
}

/** A scrollable checkbox list for choosing several groups or tags. */
export function MultiPick({
  legend,
  options,
  selected,
  onChange,
  empty,
  onCreate,
  createLabel,
}: {
  legend: string;
  options: { id: string; label: ReactNode; depth?: number }[];
  selected: string[];
  onChange: (ids: string[]) => void;
  empty: string;
  /** Lets the user add an item inline; resolves to its id, which is then selected. */
  onCreate?: (name: string) => Promise<string>;
  /** Accessible label of the inline "new item" field, e.g. "New tag name". */
  createLabel?: string;
}) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const create = async () => {
    const name = draft.trim();
    if (!onCreate || !name || busy) return;
    setBusy(true);
    setError(null);
    try {
      const id = await onCreate(name);
      onChange([...selected, id]);
      setDraft("");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't create it.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <fieldset className="flex flex-col gap-1">
      <legend className="mb-1 text-sm text-slate-300">{legend}</legend>
      {options.length === 0 ? (
        <p className="text-xs text-slate-500">{empty}</p>
      ) : (
        <div className="max-h-36 overflow-y-auto rounded border border-slate-800 p-2">
          {options.map((o) => (
            <label
              key={o.id}
              className="flex items-center gap-2 py-0.5 text-sm text-slate-200"
              style={{ paddingLeft: `${(o.depth ?? 0) * 1}rem` }}
            >
              <input
                type="checkbox"
                checked={selected.includes(o.id)}
                onChange={(e) => onChange(e.target.checked ? [...selected, o.id] : selected.filter((s) => s !== o.id))}
              />
              {o.label}
            </label>
          ))}
        </div>
      )}
      {onCreate && (
        <div className="mt-1 flex gap-1">
          <input
            aria-label={createLabel ?? `New ${legend.toLowerCase()}`}
            placeholder={createLabel ?? `New ${legend.toLowerCase()}`}
            className="min-w-0 flex-1 rounded border border-slate-700 bg-slate-900 px-2 py-1 text-sm text-slate-100"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              // Inside another form: Enter creates the item instead of submitting.
              if (e.key === "Enter") {
                e.preventDefault();
                void create();
              }
            }}
          />
          <button
            type="button"
            className="rounded border border-slate-700 px-2 py-1 text-sm text-slate-200 hover:bg-slate-800 disabled:opacity-50"
            disabled={!draft.trim() || busy}
            onClick={() => void create()}
          >
            Create
          </button>
        </div>
      )}
      {error && <p className="text-xs text-red-300">{error}</p>}
    </fieldset>
  );
}
