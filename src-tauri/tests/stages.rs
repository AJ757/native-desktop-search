// Keep the integration suite in the same order as the testing plan.
#[path = "stages/support/mod.rs"]
mod support;

#[path = "stages/stage_01_file_crawling.rs"]
mod stage_01_file_crawling;
#[path = "stages/stage_02_text_extraction.rs"]
mod stage_02_text_extraction;
#[path = "stages/stage_03_sqlite_operations.rs"]
mod stage_03_sqlite_operations;
#[path = "stages/stage_04_tantivy_indexing.rs"]
mod stage_04_tantivy_indexing;
#[path = "stages/stage_05_keyword_search.rs"]
mod stage_05_keyword_search;
#[path = "stages/stage_06_file_updates_deletion.rs"]
mod stage_06_file_updates_deletion;
#[path = "stages/stage_07_incremental_synchronization.rs"]
mod stage_07_incremental_synchronization;
#[path = "stages/stage_08_persistence_restart.rs"]
mod stage_08_persistence_restart;
#[path = "stages/stage_09_file_to_search_pipeline.rs"]
mod stage_09_file_to_search_pipeline;
