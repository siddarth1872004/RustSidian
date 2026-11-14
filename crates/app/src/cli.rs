//! Command-line interface. Every subcommand except `open` is headless.

use crate::shell::{App, Key};
use search_core::{Corpus, Query, RankWeights, Sort};
use std::path::{Path, PathBuf};
use std::time::Instant;
use vault_core::config::ObsidianConfig;
use vault_core::{path, VaultStore};

pub const USAGE: &str = "Rustsidian

USAGE:
  rustsidian                                    Show the welcome screen to pick or create a vault
  rustsidian [open] <vault> [note]              Open a vault in a window
  rustsidian index <vault>                      Build or refresh the index and print statistics
  rustsidian search <vault> <query> [--limit N] Search with Obsidian query syntax
  rustsidian export <vault> <note> [--format html|md] [-o <path>]
  rustsidian doctor <vault>                     Diagnose configuration, index and notes
  rustsidian screenshot <vault> <out.png> [--note <path>] [--size WxH] [--keys \"Ctrl+P ...\"] [--type <text>] [--run <command id>,...] [--light]
  rustsidian welcome-screenshot <out.png> [--run <action>,...] [--type <text>]
  rustsidian commands                           List command ids and default hotkeys

OPTIONS:
  --profile    Print a start-up timing breakdown (with `open` and `index`)

Not implemented in this build: `rustsidian plugin`, PDF export, `--config-dir`, portable mode.";

struct Args {
    pos: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

fn parse_args(args: &[String]) -> Args {
    const WITH_VALUE: &[&str] =
        &["--limit", "--format", "-o", "--note", "--size", "--keys", "--type", "--run"];
    let mut out = Args { pos: Vec::new(), flags: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if WITH_VALUE.contains(&a.as_str()) {
            out.flags.push((a.clone(), it.next().cloned()));
        } else if a.starts_with("--") {
            out.flags.push((a.clone(), None));
        } else {
            out.pos.push(a.clone());
        }
    }
    out
}

impl Args {
    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == name)
    }
    fn value(&self, name: &str) -> Option<&str> {
        self.flags.iter().find(|(k, _)| k == name).and_then(|(_, v)| v.as_deref())
    }
}

pub enum Outcome {
    /// Finished; exit with this code.
    Exit(i32),
    /// No vault was named: show the welcome screen to pick one.
    Welcome,
    /// Open a window on this vault, optionally with a note.
    Open { vault: PathBuf, note: Option<String>, profile: bool },
}

fn fail(msg: &str) -> Outcome {
    eprintln!("rustsidian: {msg}");
    Outcome::Exit(2)
}

/// Run a CLI invocation. Output goes to `out` so tests can capture it.
pub fn run(args: &[String], out: &mut dyn std::io::Write) -> Outcome {
    let a = parse_args(args);
    let Some(first) = a.pos.first().map(String::as_str) else {
        if !a.flag("--help") && cfg!(feature = "window") {
            return Outcome::Welcome;
        }
        let _ = writeln!(out, "{USAGE}");
        return Outcome::Exit(if a.flag("--help") { 0 } else { 2 });
    };
    let vault_arg = |i: usize| a.pos.get(i).map(PathBuf::from).filter(|p| p.is_dir());
    match first {
        "help" => {
            let _ = writeln!(out, "{USAGE}");
            Outcome::Exit(0)
        }
        "commands" => {
            for (id, name, key) in crate::shell::COMMANDS {
                let _ = writeln!(out, "{id:36} {key:22} {name}");
            }
            Outcome::Exit(0)
        }
        "plugin" => fail("the plugin system is not implemented in this build (see README)"),
        "index" => {
            let Some(root) = vault_arg(1) else { return fail("index: expected a vault folder") };
            match VaultStore::open(&root) {
                Ok((mut store, st)) => {
                    let t = Instant::now();
                    let saved = store.save_cache().unwrap_or(false);
                    let graph = store.index.graph();
                    let links: usize =
                        graph.out.iter().map(|o| o.iter().map(|x| x.1 as usize).sum::<usize>()).sum();
                    let unresolved: usize = graph.unresolved.iter().map(Vec::len).sum();
                    let _ = writeln!(out, "files: {}\nnotes: {}\nparsed: {}\nreused from index: {}\nresolved links: {links}\nunresolved link targets: {unresolved}", st.files, st.notes, st.parsed, st.reused);
                    let _ = writeln!(out, "index written: {saved}");
                    if a.flag("--profile") {
                        let _ = writeln!(
                            out,
                            "open: {:?} (walk {:?}, load index {:?}, parse {:?}); save + graph: {:?}",
                            st.total,
                            st.walk,
                            st.load_cache,
                            st.parse,
                            t.elapsed()
                        );
                    }
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("index: {e}")),
            }
        }
        "search" => {
            let (Some(root), Some(q)) = (vault_arg(1), a.pos.get(2)) else {
                return fail("search: expected <vault> <query>");
            };
            let query = match Query::parse(q) {
                Ok(q) => q,
                Err(e) => return fail(&format!("search: {e}")),
            };
            let store = match VaultStore::open(&root) {
                Ok((s, _)) => s,
                Err(e) => return fail(&format!("search: {e}")),
            };
            let limit = a.value("--limit").and_then(|v| v.parse().ok()).unwrap_or(20);
            let corpus = Corpus::build(&store.index, &root);
            for hit in
                corpus.search(&store.index, &root, &query, Sort::Relevance, RankWeights::default(), limit)
            {
                let Some(f) = store.index.entry(hit.file) else { continue };
                let _ = writeln!(out, "{}", f.path);
                let text = std::fs::read_to_string(root.join(&*f.path)).unwrap_or_default();
                for m in hit.matches.iter().take(3) {
                    let line = text[..(m.start as usize).min(text.len())].matches('\n').count() + 1;
                    let _ = writeln!(out, "  {line}: {}", search_core::snippet(&text, *m, 50).0.trim());
                }
            }
            Outcome::Exit(0)
        }
        "export" => {
            let (Some(root), Some(note)) = (vault_arg(1), a.pos.get(2)) else {
                return fail("export: expected <vault> <note>");
            };
            export(&root, note, a.value("--format").unwrap_or("html"), a.value("-o"), out)
        }
        "doctor" => {
            let Some(root) = vault_arg(1) else { return fail("doctor: expected a vault folder") };
            doctor(&root, out)
        }
        "screenshot" => {
            let (Some(root), Some(png)) = (vault_arg(1), a.pos.get(2)) else {
                return fail("screenshot: expected <vault> <out.png>");
            };
            let (w, h) = a
                .value("--size")
                .and_then(|s| s.split_once('x'))
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                .unwrap_or((1280u32, 800u32));
            let mut app = match App::open(&root) {
                Ok(app) => app,
                Err(e) => return fail(&format!("screenshot: {e}")),
            };
            if a.flag("--light") {
                app.dark = false;
            }
            if let Some(n) = a.value("--note") {
                app.open_note(n);
            }
            for k in a.value("--keys").unwrap_or("").split_whitespace() {
                match Key::parse(k) {
                    Some(key) => {
                        app.key(key);
                    }
                    None => return fail(&format!("screenshot: bad key `{k}`")),
                }
            }
            if let Some(t) = a.value("--type") {
                for c in t.chars() {
                    app.text_input(&c.to_string());
                }
            }
            for c in a.value("--run").unwrap_or("").split(',').filter(|c| !c.is_empty()) {
                app.run(c.trim());
            }
            if let Some(g) = &mut app.graph_view {
                g.settle();
            }
            let mut fonts = rustsidian_ui::Fonts::new();
            let Some(mut pm) = rustsidian_ui::Pixmap::new(w.clamp(64, 8192), h.clamp(64, 8192)) else {
                return fail("screenshot: bad size");
            };
            crate::render(&mut app, &mut fonts, &mut pm, 1.0);
            // Second frame so the perf overlay shows a warm-cache frame time.
            crate::render(&mut app, &mut fonts, &mut pm, 1.0);
            match pm.save_png(png) {
                Ok(()) => {
                    let _ = writeln!(
                        out,
                        "wrote {png} ({w}x{h}), frame {:.2} ms",
                        app.last_frame_us as f64 / 1000.0
                    );
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("screenshot: {e}")),
            }
        }
        "welcome-screenshot" => {
            let Some(png) = a.pos.get(1) else { return fail("welcome-screenshot: expected <out.png>") };
            let mut w = crate::welcome::Welcome::new();
            for c in a.value("--run").unwrap_or("").split(',').filter(|c| !c.is_empty()) {
                w.run(c.trim());
            }
            if let Some(t) = a.value("--type") {
                w.text_input(t);
            }
            let mut fonts = rustsidian_ui::Fonts::new();
            let Some(mut pm) = rustsidian_ui::Pixmap::new(1280, 800) else {
                return fail("welcome-screenshot: bad size");
            };
            w.render(&mut fonts, &mut pm, 1.0);
            match pm.save_png(png) {
                Ok(()) => {
                    let _ = writeln!(out, "wrote {png}");
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("welcome-screenshot: {e}")),
            }
        }
        "open" => match vault_arg(1) {
            Some(vault) => Outcome::Open { vault, note: a.pos.get(2).cloned(), profile: a.flag("--profile") },
            None => fail("open: expected a vault folder"),
        },
        _ => match vault_arg(0) {
            Some(vault) => Outcome::Open { vault, note: a.pos.get(1).cloned(), profile: a.flag("--profile") },
            None => fail(&format!("`{first}` is neither a command nor a folder. Run `rustsidian help`.")),
        },
    }
}

struct VaultLinks<'a> {
    store: &'a VaultStore,
    source: &'a str,
}

impl md_engine::html::LinkResolver for VaultLinks<'_> {
    fn resolve(&self, target: &str) -> Option<String> {
        let id = self.store.index.resolve(target, self.source)?;
        let f = self.store.index.entry(id)?;
        let rel = path::relative(path::parent(self.source), &f.path);
        Some(if f.kind == vault_core::FileKind::Markdown {
            format!("{}.html", path::strip_md(&rel))
        } else {
            rel
        })
    }
}

fn export(
    root: &Path,
    note: &str,
    format: &str,
    dest: Option<&str>,
    out: &mut dyn std::io::Write,
) -> Outcome {
    let store = match VaultStore::open(root) {
        Ok((s, _)) => s,
        Err(e) => return fail(&format!("export: {e}")),
    };
    let Some(id) = store.index.resolve(note, "") else {
        return fail(&format!("export: note `{note}` not found"));
    };
    let rel = store.index.entry(id).map(|f| f.path.to_string()).unwrap_or_default();
    let text = match store.read(&rel) {
        Ok((t, _)) => t,
        Err(e) => return fail(&format!("export: {e}")),
    };
    match format {
        "html" => {
            let doc = md_engine::parse(&text);
            let body = md_engine::html::render_with(&doc, &text, &VaultLinks { store: &store, source: &rel });
            let mut title = String::new();
            md_engine::html::escape(path::stem(&rel), &mut title);
            let html = format!(
                "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n<style>\nbody {{ max-width: 46em; margin: 2em auto; padding: 0 1em; font-family: system-ui, sans-serif; line-height: 1.6 }}\npre {{ background: #f4f4f4; padding: 1em; overflow: auto }}\n.callout {{ border-left: 4px solid #8a5cf5; background: #f6f3ff; padding: .5em 1em; margin: 1em 0 }}\n.callout-title {{ font-weight: 600 }}\n.is-unresolved {{ opacity: .6 }}\ntable {{ border-collapse: collapse }} td, th {{ border: 1px solid #ccc; padding: .3em .6em }}\nmark {{ background: #ffe066 }}\n</style>\n</head>\n<body>\n{body}</body>\n</html>\n"
            );
            let dest = dest
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(format!("{}.html", path::stem(&rel))));
            match vault_core::fsio::atomic_write(&dest, html.as_bytes()) {
                Ok(()) => {
                    let _ = writeln!(out, "wrote {}", dest.display());
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("export: {e}")),
            }
        }
        "md" => {
            // Markdown bundle: the note plus every attachment it embeds or links, flat in one folder.
            let dest = dest
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(format!("{}-bundle", path::stem(&rel))));
            let meta = vault_core::meta::extract(&text);
            let mut copied = 0;
            let mut edits = Vec::new();
            for l in &meta.links {
                let Some(t) = store.index.resolve(&l.target, &rel).and_then(|t| store.index.entry(t)) else {
                    continue;
                };
                if t.kind == vault_core::FileKind::Markdown {
                    continue;
                }
                let name = path::file_name(&t.path).to_string();
                if std::fs::create_dir_all(&dest)
                    .and_then(|()| std::fs::copy(root.join(&*t.path), dest.join(&name)))
                    .is_ok()
                {
                    copied += 1;
                    let text = if l.kind == vault_core::meta::LinkKind::Markdown {
                        path::percent_encode(&name)
                    } else {
                        name
                    };
                    edits.push(vault_core::TextEdit {
                        start: l.target_start as usize,
                        end: l.target_end as usize,
                        text,
                    });
                }
            }
            let bundled = vault_core::apply_edits(&text, &edits);
            match vault_core::fsio::atomic_write(&dest.join(path::file_name(&rel)), bundled.as_bytes()) {
                Ok(()) => {
                    let _ = writeln!(out, "wrote {} (1 note, {copied} attachments)", dest.display());
                    Outcome::Exit(0)
                }
                Err(e) => fail(&format!("export: {e}")),
            }
        }
        "pdf" => fail("export: PDF export is not implemented in this build; export HTML and print it"),
        other => fail(&format!("export: unknown format `{other}`")),
    }
}

fn doctor(root: &Path, out: &mut dyn std::io::Write) -> Outcome {
    let mut problems = 0;
    let mut line = |ok: bool, msg: String| {
        if !ok {
            problems += 1;
        }
        let _ = writeln!(out, "[{}] {msg}", if ok { " ok " } else { "WARN" });
    };
    match ObsidianConfig::load(root) {
        Ok(c) => {
            line(
                true,
                format!(
                    ".obsidian config readable (link format {:?}, theme {})",
                    c.link_format(),
                    c.css_theme().unwrap_or("default")
                ),
            );
            if let Some(t) = c.css_theme() {
                line(
                    c.dir.join("themes").join(t).join("theme.css").is_file(),
                    format!("theme `{t}` has a theme.css"),
                );
            }
            for s in c.enabled_snippets() {
                let p = c.dir.join("snippets").join(format!("{s}.css"));
                match std::fs::read_to_string(&p) {
                    Ok(css) => {
                        let sheet = theme_engine::css::parse(&css);
                        line(
                            sheet.diagnostics.is_empty(),
                            format!(
                                "snippet `{s}`: {} rules, {} unsupported or invalid constructs",
                                sheet.rules.len(),
                                sheet.diagnostics.len()
                            ),
                        );
                    }
                    Err(_) => line(false, format!("snippet `{s}` is enabled but {} is missing", p.display())),
                }
            }
        }
        Err(e) => line(false, format!(".obsidian config: {e}")),
    }
    let cache = VaultStore::cache_path(root);
    line(
        !cache.exists() || vault_core::cache::load(&cache).is_some(),
        "persisted index is valid (or absent; it is rebuilt automatically)".into(),
    );
    match VaultStore::open(root) {
        Ok((mut store, st)) => {
            line(true, format!("{} files, {} notes indexed in {:?}", st.files, st.notes, st.total));
            let bad: Vec<String> = store
                .index
                .notes()
                .filter(|(_, f)| f.meta().is_some_and(|m| m.frontmatter_error.is_some()))
                .map(|(_, f)| f.path.to_string())
                .collect();
            line(
                bad.is_empty(),
                format!(
                    "{} notes with unparsable front matter{}",
                    bad.len(),
                    bad.first().map(|p| format!(" (first: {p})")).unwrap_or_default()
                ),
            );
            let g = store.index.graph();
            let unresolved: usize = g.unresolved.iter().map(Vec::len).sum();
            line(true, format!("{unresolved} unresolved link targets (informational)"));
            let tmp = root.join(".rustsidian").join("doctor-write-test");
            let writable = vault_core::fsio::atomic_write(&tmp, b"ok").is_ok();
            let _ = std::fs::remove_file(&tmp);
            line(writable, "vault is writable".into());
        }
        Err(e) => line(false, format!("vault cannot be opened: {e}")),
    }
    let fonts = rustsidian_ui::Fonts::new();
    line(fonts.font_count() > 0, format!("{} font faces available", fonts.font_count()));
    let _ = writeln!(
        out,
        "{}",
        if problems == 0 {
            "No problems found.".to_string()
        } else {
            format!("{problems} problem(s) found.")
        }
    );
    Outcome::Exit(i32::from(problems > 0))
}
