use rustsidian_app::cli::{self, Outcome};

#[cfg(feature = "window")]
mod window;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match cli::run(&args, &mut std::io::stdout()) {
        Outcome::Exit(code) => code,
        #[cfg(feature = "window")]
        Outcome::Open { vault, note, profile } => window::run(&vault, note.as_deref(), profile),
        #[cfg(not(feature = "window"))]
        Outcome::Open { .. } => {
            eprintln!("rustsidian: this build has no window support (built without the `window` feature)");
            2
        }
    };
    std::process::exit(code);
}
