//! Choosing a work profile from real folders: what a folder can and cannot
//! do to strictness, and what breaks it.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.
#![allow(clippy::disallowed_methods)] // Tests start programs directly; no window matters here.

use std::path::{Path, PathBuf};

use keyjutsu_core::store::Store;
use keyjutsu_core::work_profile::{Decision, Places, Why, confirm, decide, hand_pick, set_machine_default};

/// A scratch tree under the build directory, unique to one test: a machine
/// home, a store, and a folder to start shells in.
struct Scene {
    root: PathBuf,
    places: Places,
    store: Store,
}

impl Scene {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("work-profile").join(name);
        if root.exists() {
            // Junctions first, so removing the tree cannot follow one.
            let _ = std::process::Command::new("cmd").args(["/c", "rmdir", "/s", "/q"]).arg(&root).output();
        }
        std::fs::create_dir_all(root.join("work")).unwrap();
        let places = Places { home: root.join("home") };
        let store = Store::open(&root.join("store")).unwrap();
        Self { root, places, store }
    }

    fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.work().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn decide(&self, start: &Path) -> Decision {
        decide(&self.places, Some(&self.store), start)
    }
}

fn junction(link: &Path, target: &Path) {
    let out = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

fn chosen(d: &Decision) -> (&str, &Why) {
    match d {
        Decision::Chosen(c) => (c.profile.name(), &c.why),
        other => panic!("expected a choice, got {other:#?}"),
    }
}

fn broken(d: &Decision) -> &str {
    match d {
        Decision::Broken { reason, .. } => reason,
        other => panic!("expected a broken folder, got {other:#?}"),
    }
}

#[test]
fn with_nothing_set_it_asks_and_then_keeps_the_machine_default() {
    let s = Scene::new("no-default");
    assert!(matches!(s.decide(&s.work()), Decision::NoDefault { .. }));
    set_machine_default(&s.places, "it-operations").unwrap();
    assert_eq!(chosen(&s.decide(&s.work())), ("it-operations", &Why::Machine));
    assert!(set_machine_default(&s.places, "nonsense").is_err());
}

#[test]
fn a_stricter_folder_applies_without_asking_from_any_subfolder() {
    let s = Scene::new("stricter");
    set_machine_default(&s.places, "it-operations").unwrap();
    s.write(".keyjutsu/profile.toml", "default = \"security-response\"\n");
    std::fs::create_dir_all(s.work().join("a/b")).unwrap();
    let d = s.decide(&s.work().join("a/b"));
    let (name, why) = chosen(&d);
    assert_eq!(name, "security-response");
    assert!(matches!(why, Why::Folder { .. }), "{why:?}");
}

#[test]
fn a_less_strict_folder_needs_confirming_once_per_set_of_files() {
    let s = Scene::new("less-strict");
    set_machine_default(&s.places, "it-operations").unwrap();
    s.write(".keyjutsu/profile.toml", "default = \"developer\"\n");
    let Decision::NeedsConfirmation(u) = s.decide(&s.work()) else { panic!("should ask") };
    assert_eq!((u.folder_profile.name(), u.machine_profile.as_str()), ("developer", "it-operations"));
    confirm(&s.store, &u, "2026-10-10T12:00:00Z").unwrap();
    assert!(matches!(chosen(&s.decide(&s.work())).1, Why::ConfirmedFolder { .. }));

    // Line endings converted by a checkout do not ask again.
    s.write(".keyjutsu/profile.toml", "default = \"developer\"\r\n");
    assert!(matches!(chosen(&s.decide(&s.work())).1, Why::ConfirmedFolder { .. }));
    // Any other change does.
    s.write(".keyjutsu/profile.toml", "default = \"developer\" # still\n");
    assert!(matches!(s.decide(&s.work()), Decision::NeedsConfirmation(_)));
}

#[test]
fn a_confirmation_holds_for_every_route_to_the_same_folder() {
    let s = Scene::new("routes");
    set_machine_default(&s.places, "helpdesk").unwrap();
    s.write(".keyjutsu/profile.toml", "default = \"developer\"\n");
    let via = s.root.join("via");
    junction(&via, &s.work());
    let Decision::NeedsConfirmation(u) = s.decide(&via) else { panic!("should ask") };
    assert_eq!(u.start.typed, via);
    assert_eq!(
        Path::new(u.start.real.as_deref().unwrap()),
        s.work().canonicalize().unwrap().strip_prefix(r"\\?\").unwrap_or(&s.work())
    );
    confirm(&s.store, &u, "2026-10-10T12:00:00Z").unwrap();
    assert!(matches!(chosen(&s.decide(&s.work())).1, Why::ConfirmedFolder { .. }));
}

#[test]
fn a_folder_can_define_profiles_that_extend_the_machines() {
    let s = Scene::new("folder-profiles");
    std::fs::create_dir_all(s.places.profiles()).unwrap();
    std::fs::write(
        s.places.profiles().join("client-x.toml"),
        "name = \"client-x\"\ntitle = \"Client X\"\nextends = \"it-operations\"",
    )
    .unwrap();
    set_machine_default(&s.places, "client-x").unwrap();
    s.write(".keyjutsu/profile.toml", "default = \"client-x-repo\"\n");
    s.write(
        ".keyjutsu/profiles/repo.toml",
        "name = \"client-x-repo\"\ntitle = \"Client X repository\"\nextends = \"client-x\"",
    );
    s.write(".keyjutsu/desktop.ini", "[.ShellClassInfo]");
    s.write(".keyjutsu/profiles/Thumbs.db", "x");
    let d = s.decide(&s.work());
    let Decision::Chosen(c) = &d else { panic!("{d:#?}") };
    assert_eq!(c.profile.name(), "client-x-repo");
    assert!(c.profile.at_least_as_strict_as("client-x"));
    assert_eq!(c.leftovers, ["desktop.ini", "profiles/Thumbs.db"]);
}

#[test]
fn what_breaks_a_folder() {
    type Setup<'a> = &'a dyn Fn(&Scene);
    let cases: [(&str, Setup); 5] = [
        ("stray", &|s: &Scene| s.write(".keyjutsu/notes.txt", "hello")),
        ("parse", &|s: &Scene| s.write(".keyjutsu/profiles/bad.toml", "name = ")),
        ("clash", &|s: &Scene| {
            s.write(
                ".keyjutsu/profiles/mine.toml",
                "name = \"helpdesk\"\ntitle = \"H\"\nextends = \"developer\"",
            )
        }),
        ("unknown", &|s: &Scene| s.write(".keyjutsu/profile.toml", "default = \"nobody\"\n")),
        ("link", &|s: &Scene| {
            std::fs::create_dir_all(s.root.join("elsewhere")).unwrap();
            junction(&s.work().join(".keyjutsu").join("profiles"), &s.root.join("elsewhere"));
        }),
    ];
    for (name, setup) in cases {
        let s = Scene::new(&format!("broken-{name}"));
        set_machine_default(&s.places, "it-operations").unwrap();
        s.write(".keyjutsu/profile.toml", "default = \"it-operations\"\n");
        setup(&s);
        let d = s.decide(&s.work());
        assert!(!broken(&d).is_empty(), "{name}");
        // A hand pick gets planning going again, recorded as an override.
        let picked = hand_pick(&s.places, &d, "helpdesk").unwrap();
        assert!(matches!(picked.why, Why::HandPicked { .. }), "{name}");
    }

    // A .keyjutsu that is itself a junction.
    let s = Scene::new("broken-self");
    std::fs::create_dir_all(s.root.join("real-kj")).unwrap();
    std::fs::write(s.root.join("real-kj/profile.toml"), "default = \"developer\"\n").unwrap();
    junction(&s.work().join(".keyjutsu"), &s.root.join("real-kj"));
    assert!(broken(&s.decide(&s.work())).contains("link or junction"));
}

#[test]
fn a_starting_folder_that_cannot_be_resolved_is_broken() {
    let s = Scene::new("unresolvable");
    set_machine_default(&s.places, "developer").unwrap();
    let d = s.decide(&s.root.join("not-there"));
    assert!(broken(&d).contains("cannot be resolved"), "{d:#?}");
    let Decision::Broken { start, .. } = &d else { unreachable!() };
    assert_eq!(start.real, None);
}

#[test]
fn a_hand_pick_lasts_until_what_it_was_picked_against_changes() {
    use keyjutsu_core::work_profile::{ProfileState, ShellProfile};
    let s = Scene::new("pick");
    set_machine_default(&s.places, "it-operations").unwrap();
    s.write(".keyjutsu/profile.toml", "default = \"developer\"\n");
    let mut shell = ShellProfile::new(s.work());
    assert_eq!(shell.view(&s.places, Some(&s.store)).state, ProfileState::NeedsConfirmation);

    shell.pick(&s.places, Some(&s.store), "audit-only").unwrap();
    let v = shell.view(&s.places, Some(&s.store));
    assert_eq!((v.state, v.name.as_deref(), v.hand_picked), (ProfileState::Chosen, Some("audit-only"), true));
    assert!(
        v.choices
            .iter()
            .any(|c| c.name == "security-response" && c.extends.as_deref() == Some("it-operations"))
    );

    // The folder's files change: the pick no longer stands.
    s.write(".keyjutsu/profile.toml", "default = \"packaging\"\n");
    let v = shell.view(&s.places, Some(&s.store));
    assert_eq!(v.state, ProfileState::NeedsConfirmation, "{v:#?}");
    assert!(!v.hand_picked);

    // A new shell in the same folder starts with no pick.
    let mut other = ShellProfile::new(s.work());
    shell.pick(&s.places, Some(&s.store), "audit-only").unwrap();
    assert_eq!(other.view(&s.places, Some(&s.store)).state, ProfileState::NeedsConfirmation);
}
