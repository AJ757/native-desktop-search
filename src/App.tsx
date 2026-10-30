// import { useCallback, useEffect, useRef, useState } from "react";
// import { open } from "@tauri-apps/plugin-dialog";
// import { api } from "./api";
// import type { IndexStatus } from "./bindings/IndexStatus";
// import type { SearchHit } from "./bindings/SearchHit";
// import type { SearchRequest } from "./bindings/SearchRequest";
// import type { Source } from "./bindings/Source";
// import type { SourceStatus } from "./bindings/SourceStatus";

// /** How often sources and index status are refreshed. */
// const POLL_MS = 2000;

// const STATUS_STYLE: Record<SourceStatus, string> = {
//   idle: "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/40 dark:text-emerald-300",
//   syncing: "bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300",
//   error: "bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300",
// };

// function sourceLabel(source: Source): string {
//   if (source.kind === "one_drive") {
//     const acc =
//       (source.config["account_name"] as string) || (source.config["account_id"] as string);
//     return `OneDrive: ${acc || "Connected"}`;
//   }
//   if (source.kind === "google_drive") {
//     const acc =
//       (source.config["account_name"] as string) || (source.config["account_id"] as string);
//     return `Google Drive: ${acc || "Connected"}`;
//   }
//   if (source.kind === "notion") {
//     const ws =
//       (source.config["workspace_name"] as string) || (source.config["workspace_id"] as string);
//     return `Notion: ${ws || "Connected"}`;
//   }
//   if (source.kind === "obsidian") {
//     const path = source.config["path"];
//     return `Obsidian: ${typeof path === "string" ? path : "Vault"}`;
//   }
//   const path = source.config["path"];
//   return typeof path === "string" ? path : source.kind;
// }

// /** Readable location for a result; cloud results store provider ids, not paths. */
// function locationLabel(hit: SearchHit): string {
//   switch (hit.sourceKind) {
//     case "google_drive":
//       return `Google Drive › ${hit.title}`;
//     case "one_drive":
//       return `OneDrive › ${hit.title}`;
//     case "notion":
//       return `Notion › ${hit.title}`;
//     default:
//       return hit.location;
//   }
// }

// export default function App() {
//   const [query, setQuery] = useState("");
//   const [hits, setHits] = useState<SearchHit[]>([]);
//   const [sources, setSources] = useState<Source[]>([]);
//   const [status, setStatus] = useState<IndexStatus | null>(null);
//   const [error, setError] = useState<string | null>(null);
//   const [sourcesOpen, setSourcesOpen] = useState(false);
//   const inputRef = useRef<HTMLInputElement>(null);

//   const refresh = useCallback(async () => {
//     try {
//       const [s, st] = await Promise.all([api.listSources(), api.getIndexStatus()]);
//       setSources(s);
//       setStatus(st);
//     } catch (e) {
//       setError(String(e));
//     }
//   }, []);

//   // Keep sources and counts current while folders are crawled and watched.
//   useEffect(() => {
//     void refresh();
//     const timer = window.setInterval(() => void refresh(), POLL_MS);
//     return () => window.clearInterval(timer);
//   }, [refresh]);

//   // The global hotkey brings the window forward; put the cursor in the box.
//   useEffect(() => {
//     const focusInput = () => inputRef.current?.select();
//     window.addEventListener("focus", focusInput);
//     return () => window.removeEventListener("focus", focusInput);
//   }, []);

//   async function runSearch() {
//     console.log("Search button clicked");
//     if (!query.trim()) return;
//     setError(null);
//     const request: SearchRequest = {
//       query,
//       filters: { sourceIds: [], fileTypes: [], modifiedAfter: null, modifiedBefore: null },
//       limit: 20,
//       mode: "keyword",
//     };
//     try {
//       const res = await api.search(request);
//       setHits(res.hits);
//     } catch (e) {
//       setError(String(e));
//     }
//   }

//   async function addFolder() {
//     setError(null);
//     try {
//       const path = await open({ directory: true, multiple: false, title: "Choose a folder" });
//       if (typeof path !== "string") return; // cancelled
//       await api.addSource({ kind: "local", config: { path } });
//       await refresh();
//     } catch (e) {
//       setError(String(e));
//     }
//   }

//   async function addObsidianVault() {
//     setError(null);
//     try {
//       const path = await open({
//         directory: true,
//         multiple: false,
//         title: "Choose Obsidian Vault folder",
//       });
//       if (typeof path !== "string") return;
//       await api.addSource({ kind: "obsidian", config: { path } });
//       await refresh();
//     } catch (e) {
//       setError(String(e));
//     }
//   }

//   async function connectOAuth(provider: "microsoft" | "notion" | "google") {
//     setError(null);
//     try {
//       await api.startOAuthLogin(provider);
//       await refresh();
//     } catch (e) {
//       setError(String(e));
//     }
//   }

//   async function openLocation(hit: SearchHit) {
//     setError(null);
//     try {
//       await api.openLocation(hit.documentId);
//     } catch (e) {
//       setError(String(e));
//     }
//   }

//   async function removeSource(source: Source) {
//     setError(null);
//     try {
//       await api.removeSource(source.id);
//       await refresh();
//     } catch (e) {
//       setError(String(e));
//     }
//   }

//   return (
//     <main className="flex min-h-screen flex-col gap-6 bg-neutral-50 p-6 text-neutral-900 dark:bg-neutral-900 dark:text-neutral-100">
//       <form
//         className="flex gap-2"
//         onSubmit={(e) => {
//           e.preventDefault();
//           void runSearch();
//         }}
//       >
//         <input
//           ref={inputRef}
//           autoFocus
//           value={query}
//           onChange={(e) => setQuery(e.target.value)}
//           onKeyDown={(e) => {
//             if (e.key === "Escape") {
//               setQuery("");
//               setHits([]);
//             }
//           }}
//           placeholder="Search your files"
//           className="flex-1 rounded-lg border border-neutral-300 bg-white px-4 py-2 text-lg outline-none focus:border-violet-500 focus:ring-2 focus:ring-violet-500/30 dark:border-neutral-700 dark:bg-neutral-800"
//         />
//         <button
//           type="submit"
//           className="rounded-lg bg-violet-600 px-4 py-2 font-medium text-white hover:bg-violet-700"
//         >
//           Search
//         </button>
//       </form>

//       {error && (
//         <p className="rounded-lg bg-red-100 px-4 py-2 text-sm text-red-800 dark:bg-red-900/40 dark:text-red-300">
//           {error}
//         </p>
//       )}

//       {hits.length > 0 && (
//         <ul className="flex flex-col gap-2">
//           {hits.map((h) => (
//             <li
//               key={h.documentId}
//               className="rounded-lg border border-neutral-200 bg-white px-4 py-3 dark:border-neutral-700 dark:bg-neutral-800"
//             >
//               <p className="font-medium">{h.title}</p>
//               <p className="text-sm text-neutral-600 dark:text-neutral-400">{h.snippet}</p>
//               <button
//                 type="button"
//                 onClick={() => void openLocation(h)}
//                 title={`Open the folder containing ${h.title}`}
//                 className="block max-w-full truncate text-left text-xs text-neutral-500 underline-offset-2 hover:text-violet-600 hover:underline"
//               >
//                 {locationLabel(h)}
//               </button>
//             </li>
//           ))}
//         </ul>
//       )}

//       <section className="flex flex-col gap-3">
//         <div className="flex flex-wrap items-center justify-between gap-2">
//           <button
//             type="button"
//             onClick={() => setSourcesOpen((o) => !o)}
//             aria-expanded={sourcesOpen}
//             aria-controls="sources-panel"
//             className="flex items-center gap-2 rounded-lg font-semibold hover:text-violet-600"
//           >
//             <h2>Sources</h2>
//             <svg
//               viewBox="0 0 20 20"
//               fill="currentColor"
//               aria-hidden="true"
//               className={`h-4 w-4 transition-transform ${sourcesOpen ? "rotate-180" : ""}`}
//             >
//               <path
//                 fillRule="evenodd"
//                 d="M5.23 7.21a.75.75 0 0 1 1.06.02L10 11.06l3.71-3.83a.75.75 0 1 1 1.08 1.04l-4.25 4.39a.75.75 0 0 1-1.08 0L5.21 8.27a.75.75 0 0 1 .02-1.06Z"
//                 clipRule="evenodd"
//               />
//             </svg>
//           </button>
//           <div className="flex flex-wrap items-center gap-2">
//             <button
//               onClick={() => void addFolder()}
//               className="rounded-lg border border-neutral-300 px-3 py-1 text-sm hover:bg-neutral-100 dark:border-neutral-700 dark:hover:bg-neutral-800"
//             >
//               + Folder
//             </button>
//             <button
//               onClick={() => void addObsidianVault()}
//               className="rounded-lg border border-purple-300 px-3 py-1 text-sm text-purple-700 hover:bg-purple-50 dark:border-purple-700 dark:text-purple-300 dark:hover:bg-purple-950/30"
//             >
//               + Obsidian Vault
//             </button>
//             <button
//               onClick={() => void connectOAuth("google")}
//               className="rounded-lg border border-emerald-300 px-3 py-1 text-sm text-emerald-700 hover:bg-emerald-50 dark:border-emerald-700 dark:text-emerald-300 dark:hover:bg-emerald-950/30"
//             >
//               + Google Drive
//             </button>
//             <button
//               onClick={() => void connectOAuth("microsoft")}
//               className="rounded-lg border border-blue-300 px-3 py-1 text-sm text-blue-700 hover:bg-blue-50 dark:border-blue-700 dark:text-blue-300 dark:hover:bg-blue-950/30"
//             >
//               + OneDrive / M365
//             </button>
//             <button
//               onClick={() => void connectOAuth("notion")}
//               className="rounded-lg border border-neutral-400 px-3 py-1 text-sm text-neutral-800 hover:bg-neutral-100 dark:border-neutral-600 dark:text-neutral-200 dark:hover:bg-neutral-800"
//             >
//               + Notion
//             </button>
//           </div>
//         </div>

//         {sourcesOpen && (
//           <div id="sources-panel" className="flex flex-col gap-3">
//             {sources.length === 0 ? (
//               <p className="text-sm text-neutral-500">No folders yet. Add one to start indexing.</p>
//             ) : (
//               <ul className="flex flex-col gap-2">
//                 {sources.map((s) => (
//                   <li
//                     key={s.id}
//                     className="flex items-center gap-3 rounded-lg border border-neutral-200 bg-white px-4 py-2 dark:border-neutral-700 dark:bg-neutral-800"
//                   >
//                     <span className="min-w-0 flex-1 truncate text-sm" title={sourceLabel(s)}>
//                       {sourceLabel(s)}
//                     </span>
//                     <span className={`rounded-full px-2 py-0.5 text-xs ${STATUS_STYLE[s.status]}`}>
//                       {s.status}
//                     </span>
//                     <button
//                       onClick={() => void removeSource(s)}
//                       className="text-sm text-neutral-500 hover:text-red-600"
//                       aria-label={`Remove ${sourceLabel(s)}`}
//                     >
//                       Remove
//                     </button>
//                   </li>
//                 ))}
//               </ul>
//             )}

//             {status && (
//               <p className="text-xs text-neutral-500">
//                 {status.totalDocuments.toLocaleString()} files tracked
//                 {status.isIndexing &&
//                   ` · syncing, ${status.queued.toLocaleString()} changes queued`}
//               </p>
//             )}
//           </div>
//         )}
//       </section>
//     </main>
//   );
// }

// VERSION 2

import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import type { IndexStatus } from "./bindings/IndexStatus";
import type { SearchHit } from "./bindings/SearchHit";
import type { SearchMode } from "./bindings/SearchMode";
import type { SearchRequest } from "./bindings/SearchRequest";
import type { Source } from "./bindings/Source";
import type { SourceKind } from "./bindings/SourceKind";
import type { SourceStatus } from "./bindings/SourceStatus";

/** How often sources and index status are refreshed. */
const POLL_MS = 2000;
const RESULT_LIMIT = 20;

type Tone = "purple" | "green" | "blue" | "grey" | "alert";
type Provider = "microsoft" | "notion" | "google";

const KIND_META: Record<SourceKind, { tag: string; tone: Tone }> = {
  local: { tag: "DIR", tone: "grey" },
  obsidian: { tag: "OBS", tone: "purple" },
  google_drive: { tag: "GDR", tone: "green" },
  one_drive: { tag: "ODR", tone: "blue" },
  notion: { tag: "NTN", tone: "grey" },
};

const STATUS_META: Record<SourceStatus, { label: string; tone: Tone }> = {
  idle: { label: "READY", tone: "green" },
  syncing: { label: "SYNC", tone: "blue" },
  error: { label: "ERR", tone: "alert" },
};

const MODES: { mode: SearchMode; label: string; hint: string }[] = [
  { mode: "keyword", label: "KEY", hint: "Keyword: exact words, fastest" },
  { mode: "semantic", label: "SEM", hint: "Semantic: matches meaning" },
  { mode: "hybrid", label: "HYB", hint: "Hybrid: keyword + meaning" },
];

/** 10x10 pixel magnifier. X = purple lens, G = green glint, B = blue handle. */
const LOGO = [
  "..XXXX....",
  ".X....X...",
  "X..GG..X..",
  "X.G....X..",
  "X......X..",
  ".X....X...",
  "..XXXXBB..",
  "......BBB.",
  ".......BBB",
  "........BB",
];
const LOGO_FILL: Record<string, string> = {
  X: "var(--color-px-purple)",
  G: "var(--color-px-green)",
  B: "var(--color-px-blue)",
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

function formatDate(seconds: number | null): string {
  return seconds === null ? "" : new Date(seconds * 1000).toISOString().slice(0, 10);
}

function PixelLogo() {
  return (
    <svg
      viewBox="0 0 10 10"
      shapeRendering="crispEdges"
      aria-hidden="true"
      className="bob h-10 w-10 shrink-0"
    >
      {LOGO.flatMap((row, y) =>
        [...row].map((cell, x) =>
          cell === "." ? null : (
            <rect
              key={`${x}-${y}`}
              x={x}
              y={y}
              width={1}
              height={1}
              style={{ fill: LOGO_FILL[cell] }}
            />
          ),
        ),
      )}
    </svg>
  );
}

/** Level = log2(files indexed); the bar is progress toward the next level. */
function Hud({ status }: { status: IndexStatus | null }) {
  const total = status?.totalDocuments ?? 0;
  const level = Math.floor(Math.log2(total + 1));
  const base = 2 ** level;
  const xp = Math.round(((total + 1 - base) / base) * 100);
  const busy = status?.isIndexing ?? false;

  return (
    <div className="w-52 shrink-0" aria-label="Index status">
      <div className="flex items-baseline justify-between font-pixel text-[8px]">
        <span className="glow text-px-green">LV {String(level).padStart(2, "0")}</span>
        <span className="text-px-dim">{total.toLocaleString()} FILES</span>
      </div>
      <div className={`px-meter mt-2 ${busy ? "is-busy" : ""}`}>
        <div className="px-meter-fill" style={{ width: `${xp}%` }} />
      </div>
      <p className="mt-2 font-pixel text-[8px] text-px-dim">
        {busy ? (
          <span className="dots text-px-blue">
            SYNC · {(status?.queued ?? 0).toLocaleString()} QUEUED
          </span>
        ) : (
          "INDEX IDLE"
        )}
      </p>
    </div>
  );
}

/** 1-5 blocks, relative to the best hit of the round. */
function ScorePips({ score, max }: { score: number; max: number }) {
  const filled = max > 0 ? Math.min(5, Math.max(1, Math.round((score / max) * 5))) : 0;
  return (
    <span className="pips" title={`score ${score.toFixed(3)}`}>
      {Array.from({ length: 5 }, (_, i) => (
        <span key={i} className={i < filled ? "on" : ""} />
      ))}
    </span>
  );
}

export default function App() {
  const [query, setQuery] = useState("");
  const [mode, setMode] = useState<SearchMode>("keyword");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [meta, setMeta] = useState<{ total: number; tookMs: number } | null>(null);
  const [lastQuery, setLastQuery] = useState<string | null>(null);
  const [round, setRound] = useState(0);
  const [selected, setSelected] = useState(-1);
  const [busy, setBusy] = useState(false);
  const [filterIds, setFilterIds] = useState<number[]>([]);
  const [sources, setSources] = useState<Source[]>([]);
  const [status, setStatus] = useState<IndexStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const searchSeq = useRef(0);


  const inventoryOpen = sourcesOpen || sources.length === 0;
  const maxScore = hits.reduce((m, h) => Math.max(m, h.score), 0);


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

  //
  const oauthPending = pending === "google" || pending === "microsoft" || pending === "notion";
  //


  async function runSearch(nextMode: SearchMode = mode, nextFilter: number[] = filterIds) {
    const q = query.trim();
    if (!q) return;
    setError(null);
    setBusy(true);
    const request: SearchRequest = {
      query: q,
      filters: {
        sourceIds: nextFilter,
        fileTypes: [],
        modifiedAfter: null,
        modifiedBefore: null,
      },
      limit: RESULT_LIMIT,
      mode: nextMode,
    };
    /*try {
      const res = await api.search(request);
      setHits(res.hits);
      setMeta({ total: res.total, tookMs: res.tookMs });
      setLastQuery(q);
      setRound((r) => r + 1);
      setSelected(-1);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }*/
    const seq = ++searchSeq.current;
    try {
      const res = await api.search(request);
      if (seq !== searchSeq.current) return; // a newer search has started
      setHits(res.hits);
      setMeta({ total: res.total, tookMs: res.tookMs });
      setLastQuery(q);
      setRound((r) => r + 1);
      setSelected(-1);
    } catch (e) {
      if (seq === searchSeq.current) setError(String(e));
    } finally {
      if (seq === searchSeq.current) setBusy(false);
    }

    
  }

  function changeMode(next: SearchMode) {
    setMode(next);
    if (lastQuery !== null) void runSearch(next, filterIds);
  }

  function applyFilter(next: number[]) {
    setFilterIds(next);
    if (lastQuery !== null) void runSearch(mode, next);
  }

  function toggleFilter(id: number) {
    applyFilter(filterIds.includes(id) ? filterIds.filter((x) => x !== id) : [...filterIds, id]);
  }

  function clearSearch() {
    searchSeq.current++;//
    setQuery("");
    setHits([]);
    setMeta(null);
    setLastQuery(null);
    setSelected(-1);
  }

  function moveSelection(delta: number) {
    if (hits.length === 0) return;
    const next = Math.min(Math.max(selected + delta, 0), hits.length - 1);
    setSelected(next);
    document.getElementById(`hit-${next}`)?.scrollIntoView({ block: "nearest" });
  }

  /** Runs a source-adding task with a loading label on its button. */
  async function withPending(id: string, task: () => Promise<void>) {
    setError(null);
    setPending(id);
    try {
      await task();
      await refresh();
    } catch (e) {
      //setError(String(e));
      // Stopping the wait on purpose isn't an error worth showing.
      if (!String(e).includes("OAuth login cancelled")) setError(String(e));

    } finally {
      setPending(null);
    }
  }

  function addFolder(kind: "local" | "obsidian") {
    void withPending(kind, async () => {
      const path = await open({
        directory: true,
        multiple: false,
        title: kind === "obsidian" ? "Choose Obsidian Vault folder" : "Choose a folder",
      });
      if (typeof path !== "string") return; // cancelled
      await api.addSource({ kind, config: { path } });
    });
  }

  function connectOAuth(provider: Provider) {
    void withPending(provider, async () => {
      await api.startOAuthLogin(provider);
    });
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
      setFilterIds((ids) => ids.filter((x) => x !== source.id));
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  const adders: { id: string; label: string; tone: Tone; run: () => void }[] = [
    { id: "local", label: "+ Folder", tone: "grey", run: () => addFolder("local") },
    { id: "obsidian", label: "+ Obsidian", tone: "purple", run: () => addFolder("obsidian") },
    { id: "google", label: "+ G.Drive", tone: "green", run: () => connectOAuth("google") },
    { id: "microsoft", label: "+ OneDrive", tone: "blue", run: () => connectOAuth("microsoft") },
    { id: "notion", label: "+ Notion", tone: "grey", run: () => connectOAuth("notion") },
  ];

  return (
    <main className="crt flex h-screen flex-col gap-4 overflow-hidden p-5">
      <header className="flex items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <PixelLogo />
          <div>
            <h1 className="glow font-pixel text-base text-px-purple">
              SEEK<span className="text-px-green">.EXE</span>
            </h1>
            <p className="mt-1 text-xs text-px-dim">native desktop search</p>
          </div>
        </div>
        <Hud status={status} />
      </header>

      <form
        className="flex items-stretch gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          void runSearch();
        }}
      >
        <label className="px-box px-field tone-purple flex flex-1 items-center gap-3 bg-px-panel px-4">
          <span className="font-pixel text-xs text-px-green" aria-hidden="true">
            &gt;
          </span>
          <input
            ref={inputRef}
            autoFocus
            value={query}
            aria-label="Search query"
            placeholder="search your files..."
            onChange={(e) => {
              setQuery(e.target.value);
              setSelected(-1);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                clearSearch();
              } else if (e.key === "ArrowDown") {
                e.preventDefault();
                moveSelection(1);
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                moveSelection(-1);
              } else if (e.key === "Enter" && hits[selected]) {
                e.preventDefault();
                void openLocation(hits[selected]);
              }
            }}
            className="min-w-0 flex-1 bg-transparent py-3 text-base text-px-ink caret-px-green outline-none placeholder:text-px-dim"
          />
        </label>
        <button type="submit" disabled={busy || !query.trim()} className="px-btn tone-purple">
          {busy ? <span className="dots">Load</span> : "Go ▶"}
        </button>
      </form>

      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3" role="radiogroup" aria-label="Search mode">
          <span className="font-pixel text-[8px] text-px-dim">MODE</span>
          {MODES.map((m) => (
            <button
              key={m.mode}
              type="button"
              role="radio"
              aria-checked={mode === m.mode}
              title={m.hint}
              onClick={() => changeMode(m.mode)}
              className={`px-chip tone-blue ${mode === m.mode ? "is-on" : ""}`}
            >
              {m.label}
            </button>
          ))}
        </div>
        <div className="flex items-center gap-3 font-pixel text-[8px] text-px-dim">
          {filterIds.length > 0 && (
            <button
              type="button"
              onClick={() => applyFilter([])}
              className="text-px-blue hover:text-px-ink"
            >
              FILTER ×{filterIds.length} [CLEAR]
            </button>
          )}
          {meta && (
            <span>
              RND {String(round).padStart(2, "0")} · HITS{" "}
              <span className="text-px-green">{meta.total}</span> ·{" "}
              {(meta.tookMs / 1000).toFixed(3)}S
            </span>
          )}
        </div>
      </div>

      {error && (
        <div
          key={error}
          role="alert"
          className="shake px-box is-alert flex items-start gap-3 bg-px-panel px-4 py-2 text-sm text-px-alert"
        >
          <span className="font-pixel text-[10px]">!!</span>
          <p className="min-w-0 flex-1 break-words">{error}</p>
          <button
            type="button"
            onClick={() => setError(null)}
            aria-label="Dismiss error"
            className="font-pixel text-[10px] hover:text-px-ink"
          >
            ✕
          </button>
        </div>
      )}

      <section className="min-h-0 flex-1 overflow-y-auto py-2 pr-3 pl-7" aria-live="polite">
        {lastQuery === null ? (
          <div className="flex h-full flex-col items-center justify-center gap-4 text-center">
            <p className="blink glow font-pixel text-base text-px-purple">PRESS START</p>
            <p className="text-sm text-px-dim">
              {sources.length === 0
                ? "No sources yet. Add a folder in INVENTORY below to begin."
                : "Type a query and hit ENTER."}
            </p>
          </div>
        ) : hits.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-4 text-center">
            <p className="glow font-pixel text-base text-px-purple">GAME OVER</p>
            <p className="text-sm text-px-dim">No hits for “{lastQuery}”.</p>
            <p className="dots font-pixel text-[8px] text-px-green">
              CONTINUE? TRY {mode === "hybrid" ? "ANOTHER WORD" : "HYB MODE"}
            </p>
          </div>
        ) : (
          <ul className="flex flex-col gap-4">
            {hits.map((h, i) => {
              const kind = KIND_META[h.sourceKind];
              return (
                <li
                  key={`${round}-${h.documentId}`}
                  id={`hit-${i}`}
                  className="pop-in"
                  style={{ animationDelay: `${i * 50}ms` }}
                >
                  <button
                    type="button"
                    onClick={() => void openLocation(h)}
                    onMouseEnter={() => setSelected(i)}
                    title={`Open the folder containing ${h.title}`}
                    className={`hit px-box flex w-full items-center gap-3 px-3 py-2 text-left ${
                      i === selected ? "is-selected" : ""
                    }`}
                  >
                    <span className={`badge tone-${kind.tone}`}>{kind.tag}</span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-bold text-px-ink">{h.title}</span>
                      {h.snippet && (
                        <span className="line-clamp-2 text-sm text-px-dim">{h.snippet}</span>
                      )}
                      <span className="block truncate text-xs text-px-steel">
                        {locationLabel(h)}
                      </span>
                    </span>
                    <span className="flex shrink-0 flex-col items-end gap-2">
                      <ScorePips score={h.score} max={maxScore} />
                      <span className="text-[10px] text-px-dim">{formatDate(h.modifiedAt)}</span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <section className="px-box bg-px-panel">
        <div className="flex flex-wrap items-center justify-between gap-2 px-3 py-2">
          <button
            type="button"
            onClick={() => setSourcesOpen((o) => !o)}
            aria-expanded={inventoryOpen}
            aria-controls="sources-panel"
            className="font-pixel text-[10px] text-px-ink hover:text-px-purple"
          >
            {inventoryOpen ? "▼" : "▶"} INVENTORY{" "}
            <span className="text-px-dim">[{sources.length}]</span>
          </button>
          <p className="text-[11px] text-px-dim">
            ↑↓ select · ⏎ open · esc clear · ctrl+shift+space toggle
          </p>
        </div>

        {inventoryOpen && (
          <div id="sources-panel" className="flex flex-col gap-3 border-t-[3px] border-px-line p-3">
          
            {/*<div className="flex flex-wrap gap-3">
              {adders.map((a) => (
                <button
                  key={a.id}
                  type="button"
                  onClick={a.run}
                  disabled={pending !== null}
                  className={`px-btn px-btn--sm tone-${a.tone}`}
                >
                  {pending === a.id ? <span className="dots">Wait</span> : a.label}
                </button>
              ))}
            </div>*/}
            <div className="flex flex-wrap gap-3">
              {adders.map((a) => {
                // The waiting OAuth button doubles as the stop button.
                const stoppable = pending === a.id && oauthPending;
                return (
                  <button
                    key={a.id}
                    type="button"
                    onClick={stoppable ? () => void api.cancelOAuthLogin() : a.run}
                    disabled={pending !== null && !stoppable}
                    title={stoppable ? "Stop waiting for the browser login" : undefined}
                    className={`px-btn px-btn--sm ${stoppable ? "tone-alert" : `tone-${a.tone}`}`}
                  >
                    {stoppable ? (
                      "■ Stop wait"
                    ) : pending === a.id ? (
                      <span className="dots">Wait</span>
                    ) : (
                      a.label
                    )}
                  </button>
                );
              })}
            </div>


            {sources.length === 0 ? (
              <p className="text-sm text-px-dim">
                Inventory empty. Add a source to start indexing.
              </p>
            ) : (
              <ul className="flex max-h-36 flex-col gap-1 overflow-y-auto pr-1">
                {sources.map((s) => {
                  const kind = KIND_META[s.kind];
                  const st = STATUS_META[s.status];
                  const on = filterIds.includes(s.id);
                  return (
                    <li
                      key={s.id}
                      className="flex items-center gap-3 px-2 py-1 hover:bg-px-panel-2"
                    >
                      <button
                        type="button"
                        onClick={() => toggleFilter(s.id)}
                        aria-pressed={on}
                        title="Search only checked sources"
                        className={`font-pixel text-[10px] hover:text-px-ink ${
                          on ? "text-px-green" : "text-px-dim"
                        }`}
                      >
                        {on ? "[■]" : "[ ]"}
                      </button>
                      <span className={`badge tone-${kind.tone}`}>{kind.tag}</span>
                      <span className="min-w-0 flex-1 truncate text-sm" title={sourceLabel(s)}>
                        {sourceLabel(s)}
                      </span>
                      <span
                        className={`status tone-${st.tone} font-pixel text-[8px] ${
                          s.status === "syncing" ? "blink" : ""
                        }`}
                      >
                        {st.label}
                      </span>
                      <button
                        type="button"
                        onClick={() => void removeSource(s)}
                        aria-label={`Remove ${sourceLabel(s)}`}
                        className="font-pixel text-[10px] text-px-dim hover:text-px-alert"
                      >
                        ✕
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        )}
      </section>
    </main>
  );
}
