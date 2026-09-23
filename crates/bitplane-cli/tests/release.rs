//! The release plumbing, held to what it promised.
//!
//! `dist-workspace.toml` is the single source of the channel set: the targets,
//! the installers, and the names a user types. Three other files repeat parts
//! of it — the generated release workflow, the pull-request checks in
//! `rust.yml`, and the install page people actually read — and each repetition
//! is somewhere the truth can rot quietly.
//!
//! So each is asserted against the config rather than against a list kept here.
//! The one thing stated in this file is **ADR-0001's four targets**, because
//! that is a decision and not a derivation: adding a fifth means amending the
//! ADR, and this test is where that costs a line.

use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item};

/// ADR-0001's target table, verbatim. Every other list of targets in the repo
/// is checked against the config, and the config is checked against this.
const TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
];

/// The two targets ADR-0001 requires to carry no runtime shared library.
const STATIC_TARGETS: [&str; 2] = ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"];

#[test]
fn dist_builds_exactly_the_targets_the_adr_named() {
    let mut targets = strings(&config()["dist"]["targets"]);
    let mut expected: Vec<String> = TARGETS.iter().map(|t| (*t).to_owned()).collect();
    targets.sort();
    expected.sort();

    assert_eq!(
        targets, expected,
        "dist-workspace.toml builds a different set of targets than ADR-0001's table.\n\
         Changing the set is an ADR amendment, not a config edit."
    );
}

/// The channel list is a promise the install page makes. Two of the four
/// channels are *absences*, and an absence is the kind of thing that gets added
/// by a `dist init` rerun without anybody deciding to.
#[test]
fn the_channels_are_the_ones_the_project_committed_to() {
    let installers = strings(&config()["dist"]["installers"]);

    assert!(
        installers.iter().any(|name| name == "shell"),
        "the `curl | sh` installer is a committed channel and is not configured"
    );
    assert!(
        installers.iter().any(|name| name == "npm"),
        "the npm wrapper is a committed channel and is not configured"
    );
    assert!(
        !installers.iter().any(|name| name == "homebrew"),
        "a Homebrew tap is deliberately not set up: it has to be kept alive for \
         as long as anyone uses it, and it gets made once there are users asking \
         for it. Turning it on is a decision, not a config default."
    );
}

/// ADR-0001 accepted that the `bitplane` name stays first-come on crates.io.
/// Intent is not a mechanism; `publish = false` is.
#[test]
fn nothing_can_be_published_to_crates_io() {
    for crate_name in ["bitplane-core", "bitplane-cli"] {
        let manifest = document(&crate_manifest(crate_name));

        assert_eq!(
            manifest["package"]["publish"].as_bool(),
            Some(false),
            "{crate_name} may be published to crates.io, which ADR-0001 says it may not"
        );
    }

    // dist has no crates.io publisher, so this is not what stops one; it is
    // here because `publish-jobs` is where a *new* package manager would be
    // switched on, and each one is a channel somebody has to keep alive.
    assert_eq!(
        strings(&config()["dist"]["publish-jobs"]),
        vec!["npm".to_owned()],
        "dist publishes to something other than npm"
    );
}

/// Building the npm package only attaches a tarball to the release. Publishing
/// it is what makes the command on the install page mean anything.
#[test]
fn the_npm_channel_is_published_and_not_merely_built() {
    assert!(
        strings(&config()["dist"]["installers"]).contains(&"npm".to_owned()),
        "the npm package is not built"
    );
    assert!(
        strings(&config()["dist"]["publish-jobs"]).contains(&"npm".to_owned()),
        "the npm package is built and never published, so `npm install -g` \
         would find nothing"
    );
    assert!(
        read(&workflow("release.yml")).contains("NPM_TOKEN"),
        "release.yml has no npm publish step"
    );
}

/// The whole point of doing this ticket early: a dependency that pulls in a
/// shared library has to fail on the pull request that adds it, not at the tag.
#[test]
fn every_target_is_built_on_every_pull_request() {
    let release = read(&workflow("release.yml"));

    assert!(
        release.contains("\n  pull_request:\n"),
        ".github/workflows/release.yml does not run on pull requests"
    );
    assert_eq!(
        config()["dist"]["pr-run-mode"].as_str(),
        Some("upload"),
        "pr-run-mode is not `upload`, so pull requests only plan the release \
         instead of building it — and a target that stopped compiling would \
         first be noticed at the tag"
    );
}

/// Compiling is not linking. A musl build that quietly grew an interpreter and
/// a `NEEDED` entry compiles perfectly well, so the constraint is checked
/// against the binary, and this asserts that it is checked at all.
#[test]
fn both_musl_targets_are_checked_for_static_linking() {
    let rust = read(&workflow("rust.yml"));

    for target in STATIC_TARGETS {
        assert!(
            rust.contains(target),
            ".github/workflows/rust.yml does not build {target}, so nothing \
             proves it still links statically"
        );
    }
    assert!(
        rust.contains("readelf"),
        "rust.yml builds the musl targets but never inspects the binaries, \
         which is the only part of this that is actually about linking"
    );
}

/// `release.yml` is generated, and a generator's output stops matching its
/// input the moment someone edits the input alone. CI runs
/// `dist generate --check` for the whole file; this catches the common half of
/// it without a network call.
#[test]
fn the_generated_workflow_was_generated_from_this_config() {
    let pinned = config()["dist"]["cargo-dist-version"]
        .as_str()
        .expect("dist-workspace.toml pins a dist version")
        .to_owned();
    let release = read(&workflow("release.yml"));

    assert!(
        release.contains(&format!("cargo-dist/releases/download/v{pinned}/")),
        "dist-workspace.toml pins dist {pinned} and release.yml installs some \
         other version.\n  Run `dist generate` and commit the result."
    );
}

/// The install page is the specification for this ticket, and it names the
/// files a user downloads. Those names are dist's to choose, not the page's.
#[test]
fn the_install_page_names_the_artefacts_dist_actually_produces() {
    let page = read(&repo_root().join("website/docs/install.md"));
    let app = app_name();

    assert!(
        page.contains(&format!("{app}-installer.sh")),
        "the install page's `curl` URL does not name `{app}-installer.sh`, \
         which is the file dist puts on the release.\n  \
         The app name is the cargo package name; `npm-package` renames the npm \
         channel and nothing else."
    );

    let config = config();
    let npm = config["dist"]["npm-package"]
        .as_str()
        .expect("the npm channel is published under a stated name");
    assert!(
        page.contains(&format!("npm install -g {npm}")),
        "the install page tells people to `npm install -g` something other \
         than `{npm}`, which is the name dist publishes under"
    );

    for target in TARGETS {
        assert!(
            page.contains(target),
            "the install page's platform table does not name {target}, so a \
             reader cannot tell which archive on the release is theirs"
        );
    }
}

/// A release artefact should be the binary `cargo build --release` produces,
/// not a near relative of it with different optimisation settings.
#[test]
fn the_release_profile_is_the_profile_releases_are_built_with() {
    let manifest = document(&repo_root().join("Cargo.toml"));
    let profile = &manifest["profile"]["dist"];

    assert_eq!(
        profile["inherits"].as_str(),
        Some("release"),
        "[profile.dist] does not inherit [profile.release]"
    );

    let overrides: Vec<String> = profile
        .as_table_like()
        .expect("[profile.dist] is a table")
        .iter()
        .map(|(key, _)| key.to_owned())
        .filter(|key| key != "inherits")
        .collect();

    assert!(
        overrides.is_empty(),
        "[profile.dist] overrides {overrides:?}, so what is released is not \
         what `cargo build --release` builds here"
    );
}

/// The one config every other assertion reads.
fn config() -> DocumentMut {
    document(&repo_root().join("dist-workspace.toml"))
}

/// What dist calls the app, which is what it puts in the middle of every
/// artefact name. dist takes it from the cargo package and offers no override —
/// `npm-package` renames the npm channel and nothing else — so renaming this
/// crate renames the installer, and the install page has to follow.
fn app_name() -> String {
    document(&crate_manifest("bitplane-cli"))["package"]["name"]
        .as_str()
        .expect("the CLI crate is named")
        .to_owned()
}

fn workflow(name: &str) -> PathBuf {
    repo_root().join(".github/workflows").join(name)
}

fn crate_manifest(name: &str) -> PathBuf {
    repo_root().join("crates").join(name).join("Cargo.toml")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the crate sits two levels under the repository root")
}

fn document(path: &Path) -> DocumentMut {
    read(path)
        .parse()
        .unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// An array of strings, or an empty list where the key is absent — a key that
/// is not set and a key set to nothing mean the same thing to dist.
fn strings(item: &Item) -> Vec<String> {
    item.as_array()
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
