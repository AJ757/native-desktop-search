import { invoke } from "@tauri-apps/api/core";
import type { AddSourceRequest } from "./bindings/AddSourceRequest";
import type { IndexStatus } from "./bindings/IndexStatus";
import type { SearchRequest } from "./bindings/SearchRequest";
import type { SearchResponse } from "./bindings/SearchResponse";
import type { Source } from "./bindings/Source";

export const api = {
  search: (request: SearchRequest) => invoke<SearchResponse>("search", { request }),
  addSource: (request: AddSourceRequest) => invoke<Source>("add_source", { request }),
  listSources: () => invoke<Source[]>("list_sources"),
  getIndexStatus: () => invoke<IndexStatus>("get_index_status"),
};

export type Api = typeof api; // Track C can implement a mock of this type