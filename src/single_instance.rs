use std::fmt;
use std::fs;
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum SingleInstanceError {
    MissingRuntimeDir,
    AlreadyRunning,
    Io(io::Error),
}

impl fmt::Display for SingleInstanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRuntimeDir => write!(f, "XDG_RUNTIME_DIR is not set"),
            Self::AlreadyRunning => write!(f, "another instance is already running"),
            Self::Io(err) => write!(f, "single-instance guard failed: {err}"),
        }
    }
}

impl std::error::Error for SingleInstanceError {}

impl From<io::Error> for SingleInstanceError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug)]
pub struct SingleInstance {
    path: PathBuf,
    _listener: UnixListener,
}

impl SingleInstance {
    pub fn acquire(name: &str) -> Result<Self, SingleInstanceError> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or(SingleInstanceError::MissingRuntimeDir)?;
        let path = runtime.join(format!("{name}.sock"));

        match UnixListener::bind(&path) {
            Ok(listener) => Ok(Self {
                path,
                _listener: listener,
            }),
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
                if is_live_socket(&path) {
                    Err(SingleInstanceError::AlreadyRunning)
                } else {
                    fs::remove_file(&path)?;
                    let listener = UnixListener::bind(&path)?;
                    Ok(Self {
                        path,
                        _listener: listener,
                    })
                }
            }
            Err(err) => Err(SingleInstanceError::Io(err)),
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn is_live_socket(path: &Path) -> bool {
    UnixStream::connect(path).is_ok()
}
