//! Fixed generation-selected terminal and launcher consumers.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use realm_theme::generation::GenerationStore;

/// The two MVP programs that consume one selected theme generation directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixedConsumer {
    /// The default Foot terminal.
    Terminal,
    /// The default Fuzzel launcher.
    Launcher,
}

impl FixedConsumer {
    /// Parse the exact private child-mode argument pair.
    pub fn parse_args(args: &[OsString]) -> Result<Self, String> {
        match args {
            [flag, value] if flag == "--fixed-consumer" && value == "terminal" => {
                Ok(Self::Terminal)
            }
            [flag, value] if flag == "--fixed-consumer" && value == "launcher" => {
                Ok(Self::Launcher)
            }
            _ => Err("expected exactly --fixed-consumer terminal|launcher".into()),
        }
    }

    /// Stable child-mode spelling used by the worker.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Launcher => "launcher",
        }
    }

    fn executable(self) -> &'static str {
        match self {
            Self::Terminal => "foot",
            Self::Launcher => "fuzzel",
        }
    }

    fn outputs(self) -> &'static [&'static str] {
        match self {
            Self::Terminal => &[
                "foot/foot.ini",
                "zsh/.zshrc",
                "starship.toml",
                "yazi/yazi.toml",
                "yazi/keymap.toml",
                "yazi/theme.toml",
                "btop/btop.conf",
                "btop/themes/realm.theme",
                "share/themes/realm/gtk-3.0/gtk.css",
                "share/themes/realm/gtk-4.0/gtk.css",
                "qt6ct/qt6ct.conf",
                "qt6ct/colors/realm.conf",
            ],
            Self::Launcher => &["fuzzel/fuzzel.ini"],
        }
    }
}

/// Resolve the standard configuration root used by startup and child consumers.
pub fn config_root_from_env() -> Result<PathBuf, String> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .ok_or_else(|| "no XDG_CONFIG_HOME or HOME for realm configuration".into())
}

fn config_argument(path: &Path) -> OsString {
    let mut argument = OsString::from_vec(b"--config=".to_vec());
    argument.push(path);
    argument
}

fn prepend_xdg_search_root(
    generation: &Path,
    suffix: Option<&str>,
    variable: &str,
    default: &str,
) -> Result<OsString, String> {
    let generation = generation
        .to_str()
        .filter(|value| !value.contains(':'))
        .ok_or_else(|| "generation path cannot be represented in XDG search lists".to_owned())?;
    let mut value = OsString::from(generation);
    if let Some(suffix) = suffix {
        value.push("/");
        value.push(suffix);
    }
    value.push(":");
    value.push(
        std::env::var_os(variable)
            .filter(|inherited| !inherited.is_empty())
            .unwrap_or_else(|| OsString::from(default)),
    );
    Ok(value)
}

/// Lease current for this process and replace it with the fixed consumer.
pub fn exec_from_env(consumer: FixedConsumer) -> Result<(), String> {
    let root = config_root_from_env()?;
    let store = GenerationStore::open(&root.join("realm/generated"))?;
    let selection = store.select_current()?;
    for output in consumer.outputs() {
        selection
            .read_output(output)
            .map_err(|error| format!("required output {output} is unavailable: {error}"))?;
    }
    let generation = selection.path();
    let toolkit_environment = if consumer == FixedConsumer::Terminal {
        Some((
            prepend_xdg_search_root(
                generation,
                Some("share"),
                "XDG_DATA_DIRS",
                "/usr/local/share:/usr/share",
            )?,
            prepend_xdg_search_root(generation, None, "XDG_CONFIG_DIRS", "/etc/xdg")?,
        ))
    } else {
        None
    };
    let config = config_argument(&generation.join(consumer.outputs()[0]));
    let mut command = Command::new(consumer.executable());
    command.arg(config);
    if consumer == FixedConsumer::Terminal {
        command
            .arg(OsStr::new("--override=key-bindings.spawn-terminal=none"))
            .arg(OsStr::new("zsh"))
            .env("REALM_GENERATION", generation)
            .env("ZDOTDIR", generation.join("zsh"))
            .env("STARSHIP_CONFIG", generation.join("starship.toml"))
            .env("YAZI_CONFIG_HOME", generation.join("yazi"))
            .env("GTK_THEME", "realm")
            .env("XDG_DATA_DIRS", &toolkit_environment.as_ref().unwrap().0)
            .env("QT_QPA_PLATFORMTHEME", "qt6ct")
            .env("XDG_CONFIG_DIRS", &toolkit_environment.as_ref().unwrap().1);
    }
    let error = command.exec();
    drop(selection);
    Err(format!("could not exec {}: {error}", consumer.executable()))
}
