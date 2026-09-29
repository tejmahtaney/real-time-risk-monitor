//! Prints disk usage by top-level folder under the real storage root. Backs `make
//! storage-usage`. `println!` is fine here: CLAUDE.md's "no `println!`" rule
//! exempts examples and scripts.

fn main() {
    let root = match storage::StorageRoot::resolve() {
        Ok(root) => root,
        Err(err) => {
            eprintln!("could not resolve the storage root: {err}");
            std::process::exit(1);
        }
    };
    let usage = match storage::usage_by_folder(&root) {
        Ok(usage) => usage,
        Err(err) => {
            eprintln!("could not measure storage usage: {err}");
            std::process::exit(1);
        }
    };

    println!("storage root: {}", root.path().display());
    if usage.is_empty() {
        println!("(nothing there yet)");
        return;
    }
    let mut total = 0u64;
    for (folder, bytes) in &usage {
        println!("{folder:<16} {}", human_bytes(*bytes));
        total += bytes;
    }
    println!("{:-<16} {}", "", "-".repeat(12));
    println!("{:<16} {}", "total", human_bytes(total));
}

/// A byte count as a short human-readable string, e.g. `1.23 GiB`.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}
