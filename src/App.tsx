import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import type { IndexStatus } from "./bindings/IndexStatus";
import type { SearchHit } from "./bindings/SearchHit";
import type { SearchRequest } from "./bindings/SearchRequest";
import type { Source } from "./bindings/Source";
import type { SourceStatus } from "./bindings/SourceStatus";

/** How often sources and index status are refreshed. */
const POLL_MS = 2000;

const STATUS_STYLE: Record<SourceStatus, string> = {
  idle: "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300",
  syncing: "bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300",
  error: "bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300",
};

function sourceLabel(source: Source): string {
  if (source.kind === "one_drive") {
    const acc =
      (source.config["account_name"] as string) || (source.config["account_id"] as string);
    return `OneDrive: ${acc || "Connected"}`;
  }
  if (source.kind === "google_drive") {
    const acc =
      (source.config["account_name"] as string) || (source.config["account_id"] as string);
    return `Google Drive: ${acc || "Connected"}`;
  }
  if (source.kind === "notion") {
    const ws =
      (source.config["workspace_name"] as string) || (source.config["workspace_id"] as string);
    return `Notion: ${ws || "Connected"}`;
  }
  if (source.kind === "obsidian") {
    const path = source.config["path"];
    return `Obsidian: ${typeof path === "string" ? path : "Vault"}`;
  }
  const path = source.config["path"];
  return typeof path === "string" ? path : source.kind;
}

/** Readable location for a result; cloud results store provider ids, not paths. */
function locationLabel(hit: SearchHit): string {
  switch (hit.sourceKind) {
    case "google_drive":
      return `Google Drive › ${hit.title}`;
    case "one_drive":
      return `OneDrive › ${hit.title}`;
    case "notion":
      return `Notion › ${hit.title}`;
    default:
      return hit.location;
  }
}

export default function App() {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [sources, setSources] = useState<Source[]>([]);
  const [status, setStatus] = useState<IndexStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const refresh = useCallback(async () => {
    try {
      const [s, st] = await Promise.all([api.listSources(), api.getIndexStatus()]);
      setSources(s);
      setStatus(st);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // Keep sources and counts current while folders are crawled and watched.
  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [refresh]);

  // The global hotkey brings the window forward; put the cursor in the box.
  useEffect(() => {
    const focusInput = () => inputRef.current?.select();
    window.addEventListener("focus", focusInput);
    return () => window.removeEventListener("focus", focusInput);
  }, []);

  async function runSearch() {
    console.log("Search button clicked");
    if (!query.trim()) return;
    setError(null);
    const request: SearchRequest = {
      query,
      filters: { sourceIds: [], fileTypes: [], modifiedAfter: null, modifiedBefore: null },
      limit: 20,
      mode: "keyword",
    };
    try {
      const res = await api.search(request);
      setHits(res.hits);
    } catch (e) {
      setError(String(e));
    }
  }

  async function addFolder() {
    setError(null);
    try {
      const path = await open({ directory: true, multiple: false, title: "Choose a folder" });
      if (typeof path !== "string") return; // cancelled
      await api.addSource({ kind: "local", config: { path } });
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function addObsidianVault() {
    setError(null);
    try {
      const path = await open({
        directory: true,
        multiple: false,
        title: "Choose Obsidian Vault folder",
      });
      if (typeof path !== "string") return;
      await api.addSource({ kind: "obsidian", config: { path } });
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function connectOAuth(provider: "microsoft" | "notion" | "google") {
    setError(null);
    try {
      await api.startOAuthLogin(provider);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  async function openLocation(hit: SearchHit) {
    setError(null);
    try {
      await api.openLocation(hit.documentId);
    } catch (e) {
      setError(String(e));
    }
  }

  async function removeSource(source: Source) {
    setError(null);
    try {
      await api.removeSource(source.id);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <main className="flex min-h-screen flex-col gap-6 bg-neutral-50 p-6 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-100">
      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          void runSearch();
        }}
      >
        <input
          ref={inputRef}
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setQuery("");
              setHits([]);
            }
          }}
          placeholder="Search your files"
          className="flex-1 rounded-lg border border-neutral-300 bg-white px-4 py-2 text-lg outline-none focus:border-violet-500 focus:ring-2 focus:ring-violet-500/30 dark:border-neutral-700 dark:bg-neutral-800"
        />
        <button
          type="submit"
          className="rounded-lg bg-violet-600 px-4 py-2 font-medium text-white hover:bg-violet-700"
        >
          Search
        </button>
      </form>

      {error && (
        <p className="rounded-lg bg-red-100 px-4 py-2 text-sm text-red-800 dark:bg-red-900/40 dark:text-red-300">
          {error}
        </p>
      )}

      {hits.length > 0 && (
        <ul className="flex flex-col gap-2">
          {hits.map((h) => (
            <li
              key={h.documentId}
              className="rounded-lg border border-neutral-200 bg-white px-4 py-3 dark:border-neutral-700 dark:bg-neutral-800"
            >
              <p className="font-medium">{h.title}</p>
              <p className="text-sm text-neutral-600 dark:text-neutral-400">{h.snippet}</p>
              <button
                type="button"
                onClick={() => void openLocation(h)}
                title={`Open the folder containing ${h.title}`}
                className="block max-w-full truncate text-left text-xs text-neutral-500 underline-offset-2 hover:text-violet-600 hover:underline"
              >
                {locationLabel(h)}
              </button>
            </li>
          ))}
        </ul>
      )}

      <section className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <button
            type="button"
            onClick={() => setSourcesOpen((o) => !o)}
            aria-expanded={sourcesOpen}
            aria-controls="sources-panel"
            className="flex items-center gap-2 rounded-lg font-semibold hover:text-violet-600"
          >
            <h2>Sources</h2>
            <svg
              viewBox="0 0 20 20"
              fill="currentColor"
              aria-hidden="true"
              className={`h-4 w-4 transition-transform ${sourcesOpen ? "rotate-180" : ""}`}
            >
              <path
                fillRule="evenodd"
                d="M5.23 7.21a.75.75 0 0 1 1.06.02L10 11.06l3.71-3.83a.75.75 0 1 1 1.08 1.04l-4.25 4.39a.75.75 0 0 1-1.08 0L5.21 8.27a.75.75 0 0 1 .02-1.06Z"
                clipRule="evenodd"
              />
            </svg>
          </button>
          <div className="flex flex-wrap items-center gap-2">
            <button
              onClick={() => void addFolder()}
              className="rounded-lg border border-neutral-300 px-3 py-1 text-sm hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800"
            >
              + Folder
            </button>
            <button
              onClick={() => void addObsidianVault()}
              className="rounded-lg border border-purple-300 px-3 py-1 text-sm text-purple-700 hover:bg-purple-50 dark:border-purple-700 dark:text-purple-300 dark:hover:bg-purple-950/30"
            >
              + Obsidian Vault
            </button>
            <button
              onClick={() => void connectOAuth("google")}
              className="rounded-lg border border-emerald-300 px-3 py-1 text-sm text-emerald-700 hover:bg-emerald-50 dark:border-emerald-700 dark:text-emerald-300 dark:hover:bg-emerald-950/30"
            >
              + Google Drive
            </button>
            <button
              onClick={() => void connectOAuth("microsoft")}
              className="rounded-lg border border-blue-300 px-3 py-1 text-sm text-blue-700 hover:bg-blue-50 dark:border-blue-700 dark:text-blue-300 dark:hover:bg-blue-950/30"
            >
              + OneDrive / M365
            </button>
            <button
              onClick={() => void connectOAuth("notion")}
              className="rounded-lg border border-neutral-400 px-3 py-1 text-sm text-neutral-800 hover:bg-neutral-100 dark:border-neutral-600 dark:text-neutral-200 dark:hover:bg-neutral-800"
            >
              + Notion
            </button>
          </div>
        </div>

        {sourcesOpen && (
          <div id="sources-panel" className="flex flex-col gap-3">
            {sources.length === 0 ? (
              <p className="text-sm text-neutral-500">No folders yet. Add one to start indexing.</p>
            ) : (
              <ul className="flex flex-col gap-2">
                {sources.map((s) => (
                  <li
                    key={s.id}
                    className="flex items-center gap-3 rounded-lg border border-neutral-200 bg-white px-4 py-2 dark:border-neutral-700 dark:bg-neutral-800"
                  >
                    <span className="min-w-0 flex-1 truncate text-sm" title={sourceLabel(s)}>
                      {sourceLabel(s)}
                    </span>
                    <span className={`rounded-full px-2 py-0.5 text-xs ${STATUS_STYLE[s.status]}`}>
                      {s.status}
                    </span>
                    <button
                      onClick={() => void removeSource(s)}
                      className="text-sm text-neutral-500 hover:text-red-600"
                      aria-label={`Remove ${sourceLabel(s)}`}
                    >
                      Remove
                    </button>
                  </li>
                ))}
              </ul>
            )}

            {status && (
              <p className="text-xs text-neutral-500">
                {status.totalDocuments.toLocaleString()} files tracked
                {status.isIndexing &&
                  ` · syncing, ${status.queued.toLocaleString()} changes queued`}
              </p>
            )}
          </div>
        )}
      </section>
    </main>
  );
}
