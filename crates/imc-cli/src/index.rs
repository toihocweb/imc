//! Thin CLI wrapper around the shared `imc-index` pipeline.

pub use imc_index::{run, window_chunks, IndexStats};

pub fn report(s: &IndexStats) {
    println!("Indexed with {} in {} ms", s.embedder, s.elapsed_ms);
    println!(
        "  files: {} (new {}, modified {}, deleted {}, unchanged {})",
        s.total_files, s.new_files, s.modified_files, s.deleted_files, s.unchanged_files
    );
    println!("  symbols: {}", s.symbols);
    println!("  edges:   {}", s.edges);
    println!("  chunks:  {}", s.chunks);
}