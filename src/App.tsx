import { useEffect, useState } from "react";
import { api } from "./api";
import type { SearchHit } from "./bindings/SearchHit";
import type { SearchRequest } from "./bindings/SearchRequest";
import type { Source } from "./bindings/Source";

export default function App() {
  const [query, setQuery] = useState("hello");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [sources, setSources] = useState<Source[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refreshSources = () =>
    api.listSources().then(setSources).catch((e) => setError(String(e)));

  useEffect(() => {
    refreshSources();
  }, []);

  async function runSearch() {
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

  async function addTestSource() {
    setError(null);
    try {
      await api.addSource({ kind: "local", config: { path: "C:/test" } });
      await refreshSources();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <main style={{ padding: 24, fontFamily: "sans-serif" }}>
      <h1>Native Desktop Search</h1>

      <input value={query} onChange={(e) => setQuery(e.target.value)} />
      <button onClick={runSearch}>Search</button>
      <button onClick={addTestSource}>Add test source</button>

      {error && <p style={{ color: "crimson" }}>Error: {error}</p>}

      <ul>
        {hits.map((h) => (
          <li key={h.documentId}>
            <strong>{h.title}</strong>: {h.snippet}
          </li>
        ))}
      </ul>

      <h3>Sources ({sources.length})</h3>
      <ul>
        {sources.map((s) => (
          <li key={s.id}>
            #{s.id} {s.kind} ({s.status})
          </li>
        ))}
      </ul>
    </main>
  );
}
