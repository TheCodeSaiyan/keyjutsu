//! `keyjutsu history`, `keyjutsu technique` and `keyjutsu store`: the
//! encrypted history, Techniques made from it, and clearing either.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use keyjutsu_core::fingerprint;
use keyjutsu_core::history;
use keyjutsu_core::store::{Store, default_root};
use keyjutsu_core::technique::{self, Promote};

fn open() -> Result<Store, ExitCode> {
    Store::open(&default_root()).map_err(|e| {
        eprintln!("keyjutsu: cannot open the history store: {e}");
        ExitCode::FAILURE
    })
}

fn fail(e: impl std::fmt::Display) -> ExitCode {
    eprintln!("keyjutsu: {e}");
    ExitCode::FAILURE
}

fn pairs(items: &[String]) -> Result<Vec<(String, String)>, ExitCode> {
    items
        .iter()
        .map(|p| {
            p.split_once('=').map(|(k, v)| (k.trim().to_owned(), v.to_owned())).ok_or_else(|| {
                eprintln!("keyjutsu: --param takes NAME=VALUE, got `{p}`");
                ExitCode::from(2)
            })
        })
        .collect()
}

pub fn history_list() -> ExitCode {
    let store = match open() {
        Ok(s) => s,
        Err(c) => return c,
    };
    match history::list(&store) {
        Ok(list) if list.is_empty() => {
            println!("No sessions recorded.");
            ExitCode::SUCCESS
        }
        Ok(list) => {
            for s in list {
                println!("{}  {:<18}  {}", s.id, s.outcome, s.task);
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(e),
    }
}

/// Write session `id`'s recording as asciicast: the steps from `from` to
/// `to`, or each step to its own file in `each`.
pub fn history_export(
    id: &str,
    from: Option<&str>,
    to: Option<&str>,
    out: Option<&std::path::Path>,
    each: Option<&std::path::Path>,
) -> ExitCode {
    use keyjutsu_core::recording::{IDLE_LIMIT, Recording};
    let store = match open() {
        Ok(s) => s,
        Err(c) => return c,
    };
    let recording = match history::load_recording(&store, id) {
        Ok(Some(r)) => r,
        Ok(None) => return fail(format!("session `{id}` was not recorded; run with --record to record one")),
        Err(e) => return fail(e),
    };
    let steps = recording.steps();
    let finish = |r: &Recording| -> (Recording, Vec<String>) {
        let (clean, found) = r.redacted();
        (clean.limit_idle(IDLE_LIMIT), found)
    };
    let write = |path: &std::path::Path, r: &Recording| -> Result<(), String> {
        let (r, found) = finish(r);
        std::fs::write(path, r.to_cast()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        let taken =
            if found.is_empty() { String::new() } else { format!(" (redacted: {})", found.join(", ")) };
        println!("Wrote {}{taken}", path.display());
        Ok(())
    };
    if let Some(dir) = each {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return fail(format!("cannot make {}: {e}", dir.display()));
        }
        for (n, s) in steps.iter().enumerate() {
            let cut = match recording.cut(&s.step, &s.step) {
                Ok(c) => c,
                Err(e) => return fail(e),
            };
            if let Err(e) = write(&dir.join(format!("{:02}-{}.cast", n + 1, s.step)), &cut) {
                return fail(e);
            }
        }
        return ExitCode::SUCCESS;
    }
    let Some(out) = out else { return fail("say where to write it with --out, or use --each FOLDER") };
    let whole = from.is_none() && to.is_none();
    let cut = if whole {
        Ok(recording.clone())
    } else {
        let first = from.map(str::to_owned).or_else(|| steps.first().map(|s| s.step.clone()));
        let last = to.map(str::to_owned).or_else(|| steps.last().map(|s| s.step.clone()));
        match (first, last) {
            (Some(f), Some(l)) => recording.cut(&f, &l),
            _ => Err("the recording has no steps".into()),
        }
    };
    match cut.and_then(|c| write(out, &c)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}

pub fn history_show(id: &str) -> ExitCode {
    let store = match open() {
        Ok(s) => s,
        Err(c) => return c,
    };
    let r = match history::load(&store, id) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    println!("Session {}", r.id);
    println!("  task:     {}", r.task);
    println!("  agent:    {:?} {}", r.agent.name, r.agent.version);
    println!("  finished: {}", r.finished_at);
    println!("  outcome:  {:?}", r.outcome);
    match r.snapshot() {
        Ok(s) => {
            println!("  snapshot: {} (verified)", s.snapshot_hash());
            for id in s.graph().topological_order() {
                let ran = r.checkpoint.as_ref().and_then(|c| c.runs.iter().find(|x| x.step == id));
                let mark = match ran {
                    Some(x) if x.succeeded => "ok    ",
                    Some(_) => "FAILED",
                    None => "-     ",
                };
                println!("    {mark} {id}");
            }
        }
        Err(e) => println!("  snapshot: {e}"),
    }
    ExitCode::SUCCESS
}

/// Compare this machine with the one a session was approved on.
pub fn history_recheck(id: &str) -> ExitCode {
    let store = match open() {
        Ok(s) => s,
        Err(c) => return c,
    };
    let r = match history::load(&store, id) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    let plan = r.snapshot().ok().map(|s| s.plan().clone());
    match history::recheck(&r, &fingerprint::collect(plan.as_ref())) {
        Ok(c) if c.no_fingerprint => {
            println!(
                "This session was approved without an environment record; validate it again before reuse."
            );
            ExitCode::SUCCESS
        }
        Ok(c) if c.drifts.is_empty() => {
            println!("This machine matches the one the session was approved on.");
            ExitCode::SUCCESS
        }
        Ok(c) => {
            println!("Changed since the session was approved:");
            for d in &c.drifts {
                println!(
                    "  {}: {} -> {}",
                    d.what,
                    d.before.as_deref().unwrap_or("absent"),
                    d.after.as_deref().unwrap_or("absent")
                );
            }
            println!("Steps that require revalidation: {}", c.affected.join(", "));
            ExitCode::FAILURE
        }
        Err(e) => fail(e),
    }
}

pub fn technique_promote(session: &str, name: &str, description: &str, params: &[String]) -> ExitCode {
    let store = match open() {
        Ok(s) => s,
        Err(c) => return c,
    };
    let params = match pairs(params) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let record = match history::load(&store, session) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    let promote: Vec<Promote<'_>> =
        params.iter().map(|(n, v)| Promote { name: n, description: "", value: v, pattern: None }).collect();
    let t = match technique::promote(&record, name, description, &promote, &fingerprint::now_rfc3339()) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    if let Err(e) = technique::save(&store, &t) {
        return fail(e);
    }
    println!("Technique `{}` revision {} saved.", t.id, t.revision);
    for p in &t.parameters {
        println!("  parameter {} (was {})", p.name, p.default.as_deref().unwrap_or(""));
    }
    ExitCode::SUCCESS
}

pub fn technique_list() -> ExitCode {
    let store = match open() {
        Ok(s) => s,
        Err(c) => return c,
    };
    match technique::list(&store) {
        Ok(list) if list.is_empty() => {
            println!("No Techniques yet.");
            ExitCode::SUCCESS
        }
        Ok(list) => {
            for t in list {
                let params: Vec<&str> = t.parameters.iter().map(|p| p.name.as_str()).collect();
                let origin = if t.provenance.imported { " (imported)" } else { "" };
                println!("{}  r{}  {}{origin}  [{}]", t.id, t.revision, t.name, params.join(", "));
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(e),
    }
}

/// Make a draft plan from a Technique, and say how this machine differs
/// from where it worked. It is never approved or run here.
pub fn technique_use(id: &str, params: &[String], out: &Path) -> ExitCode {
    let store = match open() {
        Ok(s) => s,
        Err(c) => return c,
    };
    let values: BTreeMap<String, String> = match pairs(params) {
        Ok(p) => p.into_iter().collect(),
        Err(c) => return c,
    };
    let t = match technique::latest(&store, id) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let draft = match technique::instantiate(&t, &values) {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    if let Err(e) = std::fs::write(out, draft.to_json()) {
        return fail(format!("cannot write {}: {e}", out.display()));
    }
    let f = technique::fit(&t, &draft, &fingerprint::collect(Some(draft.plan())));
    println!("Draft plan written to {} from {} revision {}.", out.display(), t.id, t.revision);
    if f.no_known_good {
        println!("It has no record of working on any machine KeyJutsu knows: every step must be validated.");
    } else if f.requires_revalidation.is_empty() {
        println!("This machine matches an environment it worked on.");
    } else {
        println!("This machine differs from where it worked:");
        for d in &f.drifts {
            println!(
                "  {}: {} -> {}",
                d.what,
                d.before.as_deref().unwrap_or("absent"),
                d.after.as_deref().unwrap_or("absent")
            );
        }
        println!("Steps that require revalidation: {}", f.requires_revalidation.join(", "));
        println!(
            "If validation says it cannot run here, adapt it (`keyjutsu plan revise`) and save a new revision."
        );
    }
    println!("It is a draft: validate and approve it like any other plan.");
    ExitCode::SUCCESS
}

pub fn technique_revise(id: &str, template: &Path) -> ExitCode {
    let store = match open() {
        Ok(s) => s,
        Err(c) => return c,
    };
    let t = match technique::latest(&store, id) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let plan = match std::fs::read_to_string(template)
        .map_err(|e| e.to_string())
        .and_then(|text| keyjutsu_core::plan::parse_plan(&text).map_err(|e| e.to_string()))
    {
        Ok(p) => p.into_plan(),
        Err(e) => return fail(e),
    };
    match technique::revise(&store, &t, plan, None, &fingerprint::now_rfc3339()) {
        Ok(n) => {
            println!(
                "Technique `{}` revision {} saved; revision {} is kept as it was.",
                n.id, n.revision, t.revision
            );
            ExitCode::SUCCESS
        }
        Err(e) => fail(e),
    }
}

pub fn technique_export(id: &str, out: &Path) -> ExitCode {
    let store = match open() {
        Ok(s) => s,
        Err(c) => return c,
    };
    let t = match technique::latest(&store, id) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    match std::fs::write(out, technique::export(&t)) {
        Ok(()) => {
            println!("Exported to {}, without this machine's environments or session.", out.display());
            ExitCode::SUCCESS
        }
        Err(e) => fail(e),
    }
}

pub fn technique_import(file: &Path) -> ExitCode {
    let store = match open() {
        Ok(s) => s,
        Err(c) => return c,
    };
    let t = match std::fs::read_to_string(file).map_err(|e| e.to_string()).and_then(|t| technique::import(&t))
    {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    match technique::save(&store, &t) {
        Ok(()) => {
            println!(
                "Imported `{}` revision {} as an untrusted draft: validate and approve it here before it runs.",
                t.id, t.revision
            );
            ExitCode::SUCCESS
        }
        Err(e) => fail(e),
    }
}

/// `keyjutsu store clear`: storage management.
pub fn store_clear(history_too: bool, techniques: bool, artifacts: bool) -> ExitCode {
    let store = match open() {
        Ok(s) => s,
        Err(c) => return c,
    };
    if history_too {
        let _ = store.clear(history::RECORDING_KIND);
        match store.clear(history::KIND) {
            Ok(n) => println!("Cleared {}.", crate::count(n, "session", "sessions")),
            Err(e) => return fail(e),
        }
    }
    if techniques {
        match store.clear(technique::KIND) {
            Ok(n) => println!("Cleared {}.", crate::count(n, "Technique revision", "Technique revisions")),
            Err(e) => return fail(e),
        }
    }
    if artifacts {
        let dir = keyjutsu_core::artifacts::default_store();
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => println!("Cleared staged artifacts in {}.", dir.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("No staged artifacts."),
            Err(e) => return fail(e),
        }
    }
    if !(history_too || techniques || artifacts) {
        println!("Say what to clear: --history, --techniques, --artifacts.");
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}
