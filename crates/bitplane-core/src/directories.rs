//! Where a host keeps its planes and its projects.
//!
//! Exactly one planes directory and one projects directory per host (ADR-0002),
//! each resolved by **flag > environment > config > default**.
//!
//! | directory | flag | environment | config key | default |
//! | --- | --- | --- | --- | --- |
//! | planes | `--planes-dir` | `BITPLANE_PLANES_DIR` | `planes_dir` | `~/planes` |
//! | projects | `--projects-dir` | `BITPLANE_PROJECTS_DIR` | `projects_dir` | `$XDG_DATA_HOME/bitplane/projects` |
//!
//! XDG is honoured on macOS too, rather than `~/Library/Application Support`: a
//! developer who has set `XDG_DATA_HOME` means it. The planes directory is
//! deliberately **not** under XDG — it holds the user's actual working trees and
//! is the one path a human types daily.
//!
//! The environment is a trait rather than `std::env`, so the resolution can be
//! driven from a test without touching ambient process state that every other
//! test in the binary shares.

use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

use crate::error::EngineError;

/// The three keys a config file may hold.
const LEGAL_CONFIG_KEYS: [&str; 3] = ["version", "planes_dir", "projects_dir"];

/// A host's two directories, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directories {
    planes: PathBuf,
    projects: PathBuf,
}

/// What the command line said, where it said anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirectoryOverrides {
    pub planes_dir: Option<PathBuf>,
    pub projects_dir: Option<PathBuf>,
    pub config: Option<PathBuf>,
}

/// The ambient values resolution reads.
pub trait Environment {
    /// An environment variable, absent where it is unset or empty — an empty
    /// `BITPLANE_PLANES_DIR` is someone unsetting it, not a request to use the
    /// current directory.
    fn var(&self, key: &str) -> Option<String>;

    /// The user's home directory, where there is one.
    fn home(&self) -> Option<PathBuf>;
}

/// The real environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnvironment;

impl Directories {
    /// The two directories, spelled out. The constructor a test or a future
    /// remote engine uses when there is nothing to resolve.
    pub fn new(planes: impl Into<PathBuf>, projects: impl Into<PathBuf>) -> Directories {
        Directories {
            planes: planes.into(),
            projects: projects.into(),
        }
    }

    /// Resolves both directories by flag > environment > config > default.
    pub fn resolve(
        overrides: &DirectoryOverrides,
        environment: &impl Environment,
    ) -> Result<Directories, EngineError> {
        let config = Config::read(overrides.config.as_deref(), environment)?;

        Ok(Directories {
            planes: first_of([
                overrides.planes_dir.clone(),
                environment.var("BITPLANE_PLANES_DIR").map(PathBuf::from),
                config.planes_dir,
                Some(home(environment)?.join("planes")),
            ]),
            projects: first_of([
                overrides.projects_dir.clone(),
                environment.var("BITPLANE_PROJECTS_DIR").map(PathBuf::from),
                config.projects_dir,
                Some(
                    xdg_directory(environment, "XDG_DATA_HOME", &[".local", "share"])?
                        .join("bitplane")
                        .join("projects"),
                ),
            ]),
        })
    }

    /// Where plane directories live.
    pub fn planes(&self) -> &Path {
        &self.planes
    }

    /// Where project directories live.
    pub fn projects(&self) -> &Path {
        &self.projects
    }

    /// The directory of the plane called `id`. Its name **is** the plane's
    /// identity: the directory is the key, so renaming a plane is moving it.
    pub fn plane(&self, id: &crate::plane_id::PlaneId) -> PathBuf {
        self.planes.join(id.as_str())
    }
}

impl Environment for SystemEnvironment {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    }

    fn home(&self) -> Option<PathBuf> {
        #[allow(deprecated)]
        std::env::home_dir()
    }
}

/// `config.toml`, which may not be there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Config {
    planes_dir: Option<PathBuf>,
    projects_dir: Option<PathBuf>,
}

impl Config {
    /// Reads the config file, resolving its own path by flag > environment >
    /// default. A missing file is not an error; an unknown key in one is.
    fn read(
        override_path: Option<&Path>,
        environment: &impl Environment,
    ) -> Result<Config, EngineError> {
        let path = match override_path {
            Some(path) => path.to_path_buf(),
            None => match environment.var("BITPLANE_CONFIG") {
                Some(path) => PathBuf::from(path),
                None => xdg_directory(environment, "XDG_CONFIG_HOME", &[".config"])?
                    .join("bitplane")
                    .join("config.toml"),
            },
        };

        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(err) => return Err(EngineError::io(&path, err)),
        };

        Config::parse(&path, &text, environment)
    }

    fn parse(
        path: &Path,
        text: &str,
        environment: &impl Environment,
    ) -> Result<Config, EngineError> {
        let document: DocumentMut = text
            .parse()
            .map_err(|err: toml_edit::TomlError| config_error(path, err.message()))?;

        for (key, _) in document.iter() {
            if !LEGAL_CONFIG_KEYS.contains(&key) {
                return Err(config_error(path, format!("unknown key {key:?}")));
            }
        }

        Ok(Config {
            planes_dir: directory(path, &document, "planes_dir", environment)?,
            projects_dir: directory(path, &document, "projects_dir", environment)?,
        })
    }
}

fn directory(
    path: &Path,
    document: &DocumentMut,
    key: &str,
    environment: &impl Environment,
) -> Result<Option<PathBuf>, EngineError> {
    let Some(item) = document.get(key) else {
        return Ok(None);
    };
    let value = item
        .as_str()
        .ok_or_else(|| config_error(path, format!("{key} must be a string")))?;

    Ok(Some(expand_home(value, environment)?))
}

/// Expands a leading `~/` in a **config** value.
///
/// Config is the one file where it is answerable: `plane.toml` is read over SSH
/// and "whose home" has no answer there, so its paths are stored literally
/// (ADR-0008).
fn expand_home(value: &str, environment: &impl Environment) -> Result<PathBuf, EngineError> {
    match value.strip_prefix("~/") {
        Some(rest) => Ok(home(environment)?.join(rest)),
        None if value == "~" => home(environment),
        None => Ok(PathBuf::from(value)),
    }
}

/// An XDG base directory, honoured on macOS as well as Linux.
fn xdg_directory(
    environment: &impl Environment,
    variable: &str,
    fallback: &[&str],
) -> Result<PathBuf, EngineError> {
    if let Some(configured) = environment.var(variable) {
        return Ok(PathBuf::from(configured));
    }

    Ok(fallback
        .iter()
        .fold(home(environment)?, |path, segment| path.join(segment)))
}

fn home(environment: &impl Environment) -> Result<PathBuf, EngineError> {
    environment
        .home()
        .ok_or_else(|| EngineError::InvalidRequest {
            message: "there is no home directory to resolve bitplane's directories against"
                .to_owned(),
        })
}

fn config_error(path: &Path, message: impl Into<String>) -> EngineError {
    EngineError::ParseError {
        path: path.to_path_buf(),
        message: message.into(),
        legal_keys: LEGAL_CONFIG_KEYS
            .iter()
            .map(|key| (*key).to_owned())
            .collect(),
    }
}

fn first_of<const N: usize>(candidates: [Option<PathBuf>; N]) -> PathBuf {
    candidates
        .into_iter()
        .flatten()
        .next()
        .expect("the last candidate is always a default")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_dir;
    use std::collections::HashMap;

    #[test]
    fn the_defaults_are_a_visible_planes_directory_and_an_xdg_projects_directory() {
        let environment = FakeEnvironment::at("/home/alfonz");

        let directories =
            Directories::resolve(&DirectoryOverrides::default(), &environment).unwrap();

        assert_eq!(directories.planes(), Path::new("/home/alfonz/planes"));
        assert_eq!(
            directories.projects(),
            Path::new("/home/alfonz/.local/share/bitplane/projects")
        );
    }

    #[test]
    fn xdg_is_honoured_when_it_is_set() {
        let environment = FakeEnvironment::at("/home/alfonz").with("XDG_DATA_HOME", "/data");

        let directories =
            Directories::resolve(&DirectoryOverrides::default(), &environment).unwrap();

        assert_eq!(
            directories.projects(),
            Path::new("/data/bitplane/projects"),
            "a developer who has set XDG_DATA_HOME means it"
        );
    }

    #[test]
    fn the_environment_beats_the_default() {
        let environment = FakeEnvironment::at("/home/alfonz")
            .with("BITPLANE_PLANES_DIR", "/work/planes")
            .with("BITPLANE_PROJECTS_DIR", "/work/projects");

        let directories =
            Directories::resolve(&DirectoryOverrides::default(), &environment).unwrap();

        assert_eq!(directories.planes(), Path::new("/work/planes"));
        assert_eq!(directories.projects(), Path::new("/work/projects"));
    }

    #[test]
    fn the_flag_beats_the_environment_the_config_and_the_default() {
        let dir = scratch_dir("directories-precedence");
        let config = dir.join("config.toml");
        std::fs::write(&config, "version = 1\nplanes_dir = \"/from-config\"\n").unwrap();

        let environment =
            FakeEnvironment::at("/home/alfonz").with("BITPLANE_PLANES_DIR", "/from-environment");
        let overrides = DirectoryOverrides {
            planes_dir: Some(PathBuf::from("/from-the-flag")),
            config: Some(config),
            ..DirectoryOverrides::default()
        };

        let directories = Directories::resolve(&overrides, &environment).unwrap();

        assert_eq!(directories.planes(), Path::new("/from-the-flag"));
    }

    #[test]
    fn the_environment_beats_the_config() {
        let dir = scratch_dir("directories-environment-over-config");
        let config = dir.join("config.toml");
        std::fs::write(&config, "version = 1\nplanes_dir = \"/from-config\"\n").unwrap();

        let environment = FakeEnvironment::at("/home/alfonz")
            .with("BITPLANE_PLANES_DIR", "/from-environment")
            .with("BITPLANE_CONFIG", config.to_str().unwrap());

        let directories =
            Directories::resolve(&DirectoryOverrides::default(), &environment).unwrap();

        assert_eq!(directories.planes(), Path::new("/from-environment"));
    }

    #[test]
    fn the_config_beats_the_default_and_expands_a_leading_tilde() {
        let dir = scratch_dir("directories-config");
        let config = dir.join("config.toml");
        std::fs::write(&config, "version = 1\nplanes_dir = \"~/work/planes\"\n").unwrap();

        let environment =
            FakeEnvironment::at("/home/alfonz").with("BITPLANE_CONFIG", config.to_str().unwrap());

        let directories =
            Directories::resolve(&DirectoryOverrides::default(), &environment).unwrap();

        assert_eq!(directories.planes(), Path::new("/home/alfonz/work/planes"));
    }

    #[test]
    fn a_missing_config_file_is_not_an_error() {
        let environment =
            FakeEnvironment::at("/home/alfonz").with("BITPLANE_CONFIG", "/nowhere/config.toml");

        let directories =
            Directories::resolve(&DirectoryOverrides::default(), &environment).unwrap();

        assert_eq!(directories.planes(), Path::new("/home/alfonz/planes"));
    }

    #[test]
    fn an_unknown_key_in_the_config_is_a_parse_error_naming_it() {
        let dir = scratch_dir("directories-unknown-key");
        let config = dir.join("config.toml");
        std::fs::write(&config, "version = 1\nplane_dir = \"/typo\"\n").unwrap();

        let environment =
            FakeEnvironment::at("/home/alfonz").with("BITPLANE_CONFIG", config.to_str().unwrap());

        let error = Directories::resolve(&DirectoryOverrides::default(), &environment).unwrap_err();

        let envelope = error.envelope();
        assert_eq!(envelope.error, "parse_error");
        assert!(
            envelope.message.contains("plane_dir"),
            "got {}",
            envelope.message
        );
        assert_eq!(
            envelope.remedy.as_deref(),
            Some("Legal keys are version, planes_dir and projects_dir.")
        );
    }

    #[test]
    fn an_empty_environment_variable_is_unset_rather_than_the_current_directory() {
        let environment = FakeEnvironment::at("/home/alfonz").with("BITPLANE_PLANES_DIR", "");

        let directories =
            Directories::resolve(&DirectoryOverrides::default(), &environment).unwrap();

        assert_eq!(directories.planes(), Path::new("/home/alfonz/planes"));
    }

    #[test]
    fn a_plane_directory_is_the_planes_directory_plus_the_id() {
        let directories = Directories::new("/home/alfonz/planes", "/data/projects");
        let id = crate::plane_id::PlaneId::parse("bp-a3f9c2e1").unwrap();

        assert_eq!(
            directories.plane(&id),
            Path::new("/home/alfonz/planes/bp-a3f9c2e1")
        );
    }

    struct FakeEnvironment {
        home: PathBuf,
        variables: HashMap<String, String>,
    }

    impl FakeEnvironment {
        fn at(home: &str) -> FakeEnvironment {
            FakeEnvironment {
                home: PathBuf::from(home),
                variables: HashMap::new(),
            }
        }

        fn with(mut self, key: &str, value: &str) -> FakeEnvironment {
            self.variables.insert(key.to_owned(), value.to_owned());
            self
        }
    }

    impl Environment for FakeEnvironment {
        fn var(&self, key: &str) -> Option<String> {
            self.variables
                .get(key)
                .filter(|value| !value.is_empty())
                .cloned()
        }

        fn home(&self) -> Option<PathBuf> {
            Some(self.home.clone())
        }
    }
}
