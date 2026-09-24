//! How an exec command is started. `loader::exec` builds `/bin/sh -c <cmd>`
//! with the pinned environment; a [`Wrap`] may put something around it
//! before it is spawned: `doctor`'s perturbed environment, `trace`'s tracer.
//! The sandbox goes on last, so nothing can unwrap it.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::process::Command;

use crate::loader::LoadError;

/// Turns the command `exec` built into the one it spawns.
pub type Wrap = dyn Fn(Command) -> Result<Command, LoadError>;

/// `prefix... program args...`, with the command's working directory and
/// environment changes carried over.
pub fn prefixed<I, S>(command: &Command, prefix: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut prefix = prefix.into_iter();
    let first = prefix.next().expect("a prefix names a program");
    let mut out = Command::new(first);
    out.args(prefix)
        .arg(command.get_program())
        .args(command.get_args());
    if let Some(dir) = command.get_current_dir() {
        out.current_dir(dir);
    }
    for (k, v) in command.get_envs() {
        match v {
            Some(v) => out.env(k, v),
            None => out.env_remove(k),
        };
    }
    out
}

/// The environment the command would start with: the inherited one with
/// the command's own changes applied, sorted by name.
pub fn environment(command: &Command) -> BTreeMap<OsString, OsString> {
    let mut env: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    for (k, v) in command.get_envs() {
        match v {
            Some(v) => env.insert(k.to_os_string(), v.to_os_string()),
            None => env.remove(k),
        };
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixed_keeps_the_directory_and_environment() {
        let mut c = Command::new("/bin/sh");
        c.arg("-c")
            .arg("echo hi")
            .current_dir("/tmp")
            .env("A", "1")
            .env_remove("B");
        let p = prefixed(&c, ["/usr/bin/env", "-i"]);
        assert_eq!(p.get_program(), "/usr/bin/env");
        let args: Vec<&OsStr> = p.get_args().collect();
        assert_eq!(args, ["-i", "/bin/sh", "-c", "echo hi"]);
        assert_eq!(p.get_current_dir(), Some(std::path::Path::new("/tmp")));
        let envs: Vec<_> = p.get_envs().collect();
        assert!(envs.contains(&(OsStr::new("A"), Some(OsStr::new("1")))));
        assert!(envs.contains(&(OsStr::new("B"), None)));
        let env = environment(&c);
        assert_eq!(
            env.get(OsStr::new("A")).map(|v| v.as_os_str()),
            Some(OsStr::new("1"))
        );
        assert!(!env.contains_key(OsStr::new("B")));
    }
}
