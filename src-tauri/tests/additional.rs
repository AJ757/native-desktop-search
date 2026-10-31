// Keep additional regression tests in the order of the follow-up checklist.
#[path = "stages/support/mod.rs"]
mod support;

#[path = "additional/additional_01_corrupted_files.rs"]
mod additional_01_corrupted_files;
#[path = "additional/additional_02_empty_files.rs"]
mod additional_02_empty_files;
#[path = "additional/additional_03_unicode_filenames.rs"]
mod additional_03_unicode_filenames;
#[path = "additional/additional_04_duplicate_filenames.rs"]
mod additional_04_duplicate_filenames;
#[path = "additional/additional_05_large_documents.rs"]
mod additional_05_large_documents;
#[path = "additional/additional_06_search_edge_cases.rs"]
mod additional_06_search_edge_cases;
#[path = "additional/additional_07_file_watcher.rs"]
mod additional_07_file_watcher;
#[path = "additional/additional_08_concurrent_operations.rs"]
mod additional_08_concurrent_operations;
#[path = "additional/additional_09_error_handling.rs"]
mod additional_09_error_handling;
#[path = "additional/additional_10_exclusion_edge_cases.rs"]
mod additional_10_exclusion_edge_cases;
#[path = "additional/additional_11_file_extensions.rs"]
mod additional_11_file_extensions;
