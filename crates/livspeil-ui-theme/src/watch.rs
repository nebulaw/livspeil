// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Hot reload by polling file modification times.
//!
//! No platform file-watching dependency: the host calls
//! [`ConfigWatcher::poll`] (for example once per second, or on every frame;
//! the watcher rate-limits itself). File system and clock are traits so tests
//! drive both.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::error::ConfigError;
use crate::motion::MotionConfig;
use crate::tokens::Theme;

/// The file system operations the watcher needs.
pub trait ConfigFs {
    /// Modification time of `path`, or `None` if it does not exist.
    ///
    /// # Errors
    /// I/O errors other than "not found".
    fn modified(&self, path: &Path) -> io::Result<Option<SystemTime>>;
    /// Read the whole file.
    ///
    /// # Errors
    /// Any I/O error.
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
}

/// The real file system.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdFs;

impl ConfigFs for StdFs {
    fn modified(&self, path: &Path) -> io::Result<Option<SystemTime>> {
        match std::fs::metadata(path) {
            Ok(meta) => meta.modified().map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
}

/// A monotonic clock.
pub trait Clock {
    /// Time since an arbitrary fixed origin.
    fn now(&self) -> Duration;
}

/// The real monotonic clock.
#[derive(Clone, Copy, Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock {
            origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// What a watched file contains.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfigKind {
    /// `motion.toml`.
    Motion,
    /// `theme.toml`.
    Theme,
    /// Any other file, delivered as text (for example `keymap.toml`, parsed
    /// by `livspeil-ui-core`). The tag identifies it.
    Raw(&'static str),
}

/// A typed change.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigEvent {
    /// `motion.toml` changed (or was removed: then this holds the defaults).
    Motion(MotionConfig),
    /// `theme.toml` changed (or was removed: then this holds the defaults).
    Theme(Box<Theme>),
    /// A raw file changed; `None` when it was removed.
    Raw {
        /// The tag given to [`ConfigWatcher::watch`].
        tag: &'static str,
        /// New contents.
        contents: Option<String>,
    },
    /// The file changed but could not be read or parsed. The last good
    /// configuration stays in effect.
    Error {
        /// Which file.
        kind: ConfigKind,
        /// What went wrong, with location.
        error: ConfigError,
    },
}

#[derive(Debug)]
struct Watched {
    path: PathBuf,
    kind: ConfigKind,
    /// Last seen modification time (`None`: missing).
    mtime: Option<SystemTime>,
    /// Hash of the last contents read, to skip touch-only changes.
    hash: Option<u64>,
    /// Whether the file has been looked at once.
    seen: bool,
}

/// Polls watched files and yields typed [`ConfigEvent`]s.
#[derive(Debug)]
pub struct ConfigWatcher<F: ConfigFs = StdFs, C: Clock = SystemClock> {
    fs: F,
    clock: C,
    interval: Duration,
    last_poll: Option<Duration>,
    files: Vec<Watched>,
}

impl ConfigWatcher {
    /// A watcher on the real file system and clock, polling at most every
    /// 250 ms.
    #[must_use]
    pub fn new() -> Self {
        Self::with(StdFs, SystemClock::default(), Duration::from_millis(250))
    }
}

impl Default for ConfigWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl<F: ConfigFs, C: Clock> ConfigWatcher<F, C> {
    /// A watcher on an injected file system and clock.
    pub fn with(fs: F, clock: C, interval: Duration) -> Self {
        ConfigWatcher {
            fs,
            clock,
            interval,
            last_poll: None,
            files: Vec::new(),
        }
    }

    /// Watch `path` as `kind`. The first poll reports its current contents
    /// if it exists (a missing file means "defaults", which is not reported).
    pub fn watch(&mut self, path: impl Into<PathBuf>, kind: ConfigKind) {
        self.files.push(Watched {
            path: path.into(),
            kind,
            mtime: None,
            hash: None,
            seen: false,
        });
    }

    /// Watch `motion.toml` and `theme.toml` in `dir`.
    pub fn watch_dir(&mut self, dir: &Path) {
        self.watch(dir.join("motion.toml"), ConfigKind::Motion);
        self.watch(dir.join("theme.toml"), ConfigKind::Theme);
    }

    /// The injected file system (tests mutate it between polls).
    pub fn fs_mut(&mut self) -> &mut F {
        &mut self.fs
    }

    /// The injected clock.
    pub fn clock_mut(&mut self) -> &mut C {
        &mut self.clock
    }

    /// Poll if at least the interval has passed since the last poll;
    /// otherwise return no events without touching the file system.
    pub fn poll(&mut self) -> Vec<ConfigEvent> {
        let now = self.clock.now();
        if let Some(last) = self.last_poll
            && now.saturating_sub(last) < self.interval
        {
            return Vec::new();
        }
        self.poll_now()
    }

    /// Poll immediately.
    pub fn poll_now(&mut self) -> Vec<ConfigEvent> {
        self.last_poll = Some(self.clock.now());
        let mut events = Vec::new();
        for file in &mut self.files {
            let mtime = match self.fs.modified(&file.path) {
                Ok(m) => m,
                Err(e) => {
                    events.push(io_error(file, &e));
                    continue;
                }
            };
            let first = !file.seen;
            file.seen = true;
            if !first && mtime == file.mtime {
                continue;
            }
            file.mtime = mtime;
            if mtime.is_none() {
                // Removed (or never existed): fall back to defaults.
                let had_contents = file.hash.take().is_some();
                if had_contents {
                    events.push(removed_event(file.kind));
                }
                continue;
            }
            let text = match self.fs.read_to_string(&file.path) {
                Ok(t) => t,
                Err(e) => {
                    events.push(io_error(file, &e));
                    continue;
                }
            };
            let hash = hash_text(&text);
            if file.hash == Some(hash) {
                continue;
            }
            file.hash = Some(hash);
            events.push(parse_event(file.kind, text));
        }
        events
    }
}

fn hash_text(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn io_error(file: &Watched, e: &io::Error) -> ConfigEvent {
    ConfigEvent::Error {
        kind: file.kind,
        error: ConfigError {
            file: file_name(&file.path),
            line: 0,
            column: 0,
            message: e.to_string(),
        },
    }
}

fn removed_event(kind: ConfigKind) -> ConfigEvent {
    match kind {
        ConfigKind::Motion => ConfigEvent::Motion(MotionConfig::default()),
        ConfigKind::Theme => ConfigEvent::Theme(Box::default()),
        ConfigKind::Raw(tag) => ConfigEvent::Raw {
            tag,
            contents: None,
        },
    }
}

fn parse_event(kind: ConfigKind, text: String) -> ConfigEvent {
    let result = match kind {
        ConfigKind::Motion => MotionConfig::parse(&text).map(ConfigEvent::Motion),
        ConfigKind::Theme => Theme::parse(&text).map(|t| ConfigEvent::Theme(Box::new(t))),
        ConfigKind::Raw(tag) => Ok(ConfigEvent::Raw {
            tag,
            contents: Some(text),
        }),
    };
    result.unwrap_or_else(|error| ConfigEvent::Error { kind, error })
}

/// The Livspeil UI config directory: `$XDG_CONFIG_HOME/livspeil/ui` or
/// `$HOME/.config/livspeil/ui`; `None` if neither variable is set.
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("livspeil").join("ui"))
}
