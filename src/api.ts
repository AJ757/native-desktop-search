import { invoke } from "@tauri-apps/api/core";
import type { AddSourceRequest } from "./bindings/AddSourceRequest";
import type { IndexStatus } from "./bindings/IndexStatus";
import type { OAuthLoginResult } from "./bindings/OAuthLoginResult";
import type { SearchRequest } from "./bindings/SearchRequest";
import type { SearchResponse } from "./bindings/SearchResponse";
import type { Source } from "./bindings/Source";

export const api = {
  search: async (request: SearchRequest) => {
    console.log("Sending search request:", request);
    try {
      const result = await invoke<SearchResponse>("search", { request });
      console.log("Search response:", result);
      return result;
    } catch (error) {
      console.error("Search failed:", error);
      throw error;
    }
  },
  addSource: (request: AddSourceRequest) => invoke<Source>("add_source", { request }),
  removeSource: (id: number) => invoke<void>("remove_source", { id }),
  listSources: () => invoke<Source[]>("list_sources"),
  getIndexStatus: () => invoke<IndexStatus>("get_index_status"),
  startOAuthLogin: (provider: "microsoft" | "notion") =>
    invoke<OAuthLoginResult>("start_oauth_login", { provider }),
};

export type Api = typeof api; // Track C can implement a mock of this type
