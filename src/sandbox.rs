//! The exec `sandbox` flag: the command runs in an operating-system sandbox
//! that lets it read its declared input files and the system's programs and
//! libraries, list directories inside the repository, write only to a fresh
//! temporary directory (its `TMPDIR`) and `/dev/null`, and reach no network.
//! A read of anything else fails, so the region fails: `inputs=` is complete
//! or the command does not run. Trust is unchanged; a sandboxed region still
//! needs it.
//!
//! macOS: `sandbox-exec` with a deny-default profile. Linux: Landlock for the
//! file system (kernel 5.13 or later) and a seccomp filter that refuses
//! `socket(2)` and `io_uring_setup(2)`, both applied between fork and exec.
//! Where neither is available the region is an error; it never runs
//! unsandboxed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::loader::{Ctx, LoadError};

fn hard(message: impl Into<String>) -> LoadError {
    LoadError::Hard(message.into())
}

/// What the sandbox lets a command touch. Every path is canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Allowance {
    /// Trees it may read and execute from: the system's, and `PATH`'s
    /// directories outside the repository.
    system: Vec<PathBuf>,
    /// The declared input files.
    files: Vec<PathBuf>,
    /// The tree whose directories it may list: the repository root.
    listing: PathBuf,
    /// The directory it may write to, its `TMPDIR`.
    tmp: PathBuf,
    /// Other directories it may read, write and run from: a transcript's
    /// capture files and its `workdir=tmp`.
    writable: Vec<PathBuf>,
}

/// System trees a program needs to start and run.
#[cfg(target_os = "macos")]
const SYSTEM: &[&str] = &[
    "/bin",
    "/sbin",
    "/usr",
    "/System",
    "/Library",
    "/opt/homebrew",
    "/private/etc",
    "/private/var/db/dyld",
    "/private/var/db/timezone",
    "/private/var/select",
    // xcode-select's choice, which the /usr/bin shims of developer tools read.
    "/private/var/db/xcode_select_link",
];
#[cfg(not(target_os = "macos"))]
const SYSTEM: &[&str] = &[
    "/bin",
    "/sbin",
    "/usr",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/etc",
    "/proc",
    "/nix/store",
];

impl Allowance {
    fn new(
        ctx: &Ctx,
        files: &BTreeMap<Vec<u8>, PathBuf>,
        tmp: &Path,
    ) -> Result<Allowance, LoadError> {
        let listing = ctx.bound()?;
        let path = std::env::var_os("PATH").unwrap_or_default();
        let on_path = std::env::split_paths(&path).filter(|p| p.is_absolute());
        let mut system = Vec::new();
        for tree in SYSTEM
            .iter()
            .map(PathBuf::from)
            .chain(on_path)
            .filter_map(|p| p.canonicalize().ok())
        {
            if tree.starts_with(&listing) {
                continue;
            }
            if listing.starts_with(&tree) {
                around(&tree, &listing, &mut system);
            } else {
                system.push(tree);
            }
        }
        system.sort();
        system.dedup();
        Ok(Allowance {
            system,
            files: files.values().cloned().collect(),
            listing,
            tmp: tmp.to_path_buf(),
            writable: Vec::new(),
        })
    }
}

/// What of `tree`, a system or `PATH` directory that holds the repository
/// `hole` (a checkout in `/usr/src/app`), a sandbox may read: every entry
/// but the one on the way to `hole`, whose entries are taken in turn, so the
/// repository stays closed. A symlink counts as its target, and is left out
/// when that target is in the repository or holds it.
fn around(tree: &Path, hole: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(tree) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == hole {
            continue;
        }
        // `hole` is canonical, so a directory on the way to it is no symlink.
        if hole.starts_with(&path) {
            around(&path, hole, out);
            continue;
        }
        match path.canonicalize() {
            Ok(p) if !p.starts_with(hole) && !hole.starts_with(&p) => out.push(p),
            _ => {}
        }
    }
}

/// A sandbox for one run of one command, with its temporary directory,
/// removed when the sandbox is dropped.
pub struct Sandbox {
    /// Held so the directory outlives the command.
    _tmp: tempfile::TempDir,
    allowance: Allowance,
}

impl Sandbox {
    /// A sandbox that allows reading `files`, the region's declared inputs.
    pub fn new(ctx: &Ctx, files: &BTreeMap<Vec<u8>, PathBuf>) -> Result<Sandbox, LoadError> {
        let tmp = tempfile::Builder::new()
            .prefix("computed-sandbox")
            .tempdir()
            .map_err(|e| hard(format!("sandbox: temporary directory: {e}")))?;
        let canonical = tmp
            .path()
            .canonicalize()
            .map_err(|e| hard(format!("sandbox: temporary directory: {e}")))?;
        let allowance = Allowance::new(ctx, files, &canonical)?;
        Ok(Sandbox {
            _tmp: tmp,
            allowance,
        })
    }

    /// The sandbox also letting the command read, write and run from each
    /// of `dirs`, which must exist; they are taken canonical.
    pub fn writing(mut self, dirs: &[&Path]) -> Result<Sandbox, LoadError> {
        for dir in dirs {
            let dir = dir
                .canonicalize()
                .map_err(|e| hard(format!("sandbox: {}: {e}", dir.display())))?;
            self.allowance.writable.push(dir);
        }
        Ok(self)
    }

    /// The command, run inside the sandbox with `TMPDIR` set to its
    /// temporary directory.
    pub fn apply(&self, command: Command) -> Result<Command, LoadError> {
        let mut command = platform::apply(&self.allowance, command)?;
        command.env("TMPDIR", &self.allowance.tmp);
        Ok(command)
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
/// A string literal in a sandbox profile.
fn sbpl(p: &Path) -> String {
    let s = p.to_string_lossy();
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
/// The macOS profile. Deny by default: no network, no Mach services (which
/// could fetch on the command's behalf), no reads but the ones listed.
fn profile(a: &Allowance) -> String {
    let filters = |kind: &str, paths: &[PathBuf]| {
        paths
            .iter()
            .map(|p| format!(" ({kind} {})", sbpl(p)))
            .collect::<String>()
    };
    let system = filters("subpath", &a.system);
    let files = filters("literal", &a.files);
    let tmp = format!(
        "(subpath {}){}",
        sbpl(&a.tmp),
        filters("subpath", &a.writable)
    );
    format!(
        "(version 1)\n\
         (deny default)\n\
         (allow process-fork)\n\
         (allow process-exec (literal \"/\"){system}{files} {tmp})\n\
         (allow signal (target same-sandbox))\n\
         (allow process-info* (target same-sandbox))\n\
         (allow sysctl-read)\n\
         (allow file-read-metadata)\n\
         (allow file-read* (literal \"/\"){system})\n\
         (allow file-read-data (require-all (subpath {listing}) (vnode-type DIRECTORY)))\n\
         (allow file-read*{files})\n\
         (allow file-read* file-write* {tmp})\n\
         (allow file-read* file-write-data file-ioctl (literal \"/dev/null\") (literal \"/dev/dtracehelper\"))\n\
         (allow file-read* (literal \"/dev/random\") (literal \"/dev/urandom\") (literal \"/dev/zero\"))\n",
        listing = sbpl(&a.listing),
    )
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

    pub fn apply(a: &Allowance, command: Command) -> Result<Command, LoadError> {
        if !Path::new(SANDBOX_EXEC).is_file() {
            return Err(hard(format!("sandbox: {SANDBOX_EXEC} is missing")));
        }
        let profile = profile(a);
        Ok(crate::launch::prefixed(
            &command,
            [SANDBOX_EXEC, "-p", profile.as_str()],
        ))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use landlock::{
        ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset,
        RulesetAttr, RulesetCreated, RulesetCreatedAttr, RulesetStatus, Scope,
    };
    use std::io;
    use std::os::unix::process::CommandExt;

    pub fn apply(a: &Allowance, mut command: Command) -> Result<Command, LoadError> {
        let mut ruleset = Some(ruleset(a).map_err(|e| hard(format!("sandbox: {e}")))?);
        let filter = seccomp::filter().ok_or_else(|| {
            hard(
                "sandbox: no seccomp filter for this architecture, so the network cannot be closed",
            )
        })?;
        // SAFETY: between fork and exec the closure makes only system calls:
        // prctl(2) and landlock_restrict_self(2) through the landlock crate,
        // then prctl(2) with a filter built before the fork. It allocates
        // nothing and takes no lock.
        unsafe {
            command.pre_exec(move || {
                let status = ruleset
                    .take()
                    .ok_or_else(|| io::Error::other("sandbox: spawned twice"))?
                    .restrict_self()
                    .map_err(io::Error::other)?;
                if status.ruleset == RulesetStatus::NotEnforced {
                    return Err(io::Error::other("sandbox: Landlock is not enforced"));
                }
                seccomp::install(&filter)
            });
        }
        Ok(command)
    }

    /// Landlock ABI 1 (Linux 5.13) is required; later rights, such as
    /// truncation, device ioctls, TCP, abstract sockets and signals, are
    /// taken where the kernel has them.
    fn ruleset(a: &Allowance) -> Result<RulesetCreated, String> {
        let unsupported = |e: landlock::RulesetError| {
            format!(
                "Landlock is not available ({e}); the sandbox needs Linux 5.13 or later with Landlock enabled"
            )
        };
        let v1 = AccessFs::from_all(ABI::V1);
        let mut created = Ruleset::default()
            .set_compatibility(CompatLevel::HardRequirement)
            .handle_access(v1)
            .map_err(unsupported)?
            .set_compatibility(CompatLevel::BestEffort)
            .handle_access(AccessFs::from_all(ABI::V9))
            .and_then(|r| r.handle_access(AccessNet::from_all(ABI::V9)))
            .and_then(|r| r.scope(Scope::from_all(ABI::V9)))
            .map_err(|e| e.to_string())?
            .create()
            .map_err(unsupported)?;
        let rule = |path: &Path, access: landlock::BitFlags<AccessFs>| {
            PathFd::new(path)
                .map(|fd| PathBeneath::new(fd, access))
                .map_err(|e| format!("{}: {e}", path.display()))
        };
        let read = AccessFs::from_read(ABI::V9);
        for dir in &a.system {
            created = created
                .add_rule(rule(dir, read)?)
                .map_err(|e| e.to_string())?;
        }
        for file in &a.files {
            created = created
                .add_rule(rule(file, AccessFs::ReadFile | AccessFs::Execute)?)
                .map_err(|e| e.to_string())?;
        }
        created = created
            .add_rule(rule(&a.listing, AccessFs::ReadDir.into())?)
            .map_err(|e| e.to_string())?
            .add_rule(rule(&a.tmp, AccessFs::from_all(ABI::V9))?)
            .map_err(|e| e.to_string())?;
        for dir in &a.writable {
            created = created
                .add_rule(rule(dir, AccessFs::from_all(ABI::V9))?)
                .map_err(|e| e.to_string())?;
        }
        let devices = [
            ("/dev/null", AccessFs::ReadFile | AccessFs::WriteFile),
            ("/dev/zero", AccessFs::ReadFile.into()),
            ("/dev/random", AccessFs::ReadFile.into()),
            ("/dev/urandom", AccessFs::ReadFile.into()),
        ];
        for (dev, access) in devices {
            created = created
                .add_rule(rule(Path::new(dev), access)?)
                .map_err(|e| e.to_string())?;
        }
        Ok(created)
    }

    /// A classic BPF program for seccomp: on this architecture's native
    /// system calls, `socket(2)` and `io_uring_setup(2)` fail with EACCES
    /// and everything else is allowed. A call through another ABI (x32,
    /// i386) fails, so `socketcall(2)` cannot go round it.
    mod seccomp {
        use std::io;

        const LD_W_ABS: u16 = 0x20;
        const JEQ_K: u16 = 0x15;
        const JGE_K: u16 = 0x35;
        const RET_K: u16 = 0x06;
        const ALLOW: u32 = 0x7fff_0000;
        const ERRNO: u32 = 0x0005_0000;
        /// Offsets into `struct seccomp_data`.
        const NR: u32 = 0;
        const ARCH: u32 = 4;

        #[cfg(target_arch = "x86_64")]
        const AUDIT_ARCH: Option<u32> = Some(0xc000_003e);
        #[cfg(target_arch = "aarch64")]
        const AUDIT_ARCH: Option<u32> = Some(0xc000_00b7);
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        const AUDIT_ARCH: Option<u32> = None;

        /// x32 system calls carry this bit on an x86_64 kernel.
        const X32: u32 = 0x4000_0000;

        fn op(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
            libc::sock_filter { code, jt, jf, k }
        }

        pub fn filter() -> Option<Vec<libc::sock_filter>> {
            let arch = AUDIT_ARCH?;
            let deny = ERRNO | libc::EACCES as u32;
            Some(vec![
                op(LD_W_ABS, 0, 0, ARCH),
                op(JEQ_K, 1, 0, arch),
                op(RET_K, 0, 0, deny),
                op(LD_W_ABS, 0, 0, NR),
                op(JGE_K, 3, 0, X32),
                op(JEQ_K, 2, 0, libc::SYS_socket as u32),
                op(JEQ_K, 1, 0, libc::SYS_io_uring_setup as u32),
                op(RET_K, 0, 0, ALLOW),
                op(RET_K, 0, 0, deny),
            ])
        }

        /// Installs the filter on the calling thread. Only system calls.
        pub fn install(filter: &[libc::sock_filter]) -> io::Result<()> {
            let prog = libc::sock_fprog {
                len: filter.len() as u16,
                filter: filter.as_ptr() as *mut libc::sock_filter,
            };
            // SAFETY: prctl(2) with a pointer to a program that outlives the call.
            let failed = unsafe {
                libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                    || libc::prctl(
                        libc::PR_SET_SECCOMP,
                        libc::SECCOMP_MODE_FILTER,
                        &prog as *const libc::sock_fprog,
                    ) != 0
            };
            if failed {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    use super::*;

    pub fn apply(_: &Allowance, _: Command) -> Result<Command, LoadError> {
        Err(hard("sandbox: only macOS and Linux have a sandbox"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowance() -> Allowance {
        Allowance {
            system: vec![PathBuf::from("/usr"), PathBuf::from("/opt/my \"tools\"")],
            files: vec![PathBuf::from("/repo/docs/a.md")],
            listing: PathBuf::from("/repo"),
            tmp: PathBuf::from("/tmp/sb"),
            writable: Vec::new(),
        }
    }

    #[test]
    fn the_profile_denies_by_default_and_allows_what_is_declared() {
        let p = profile(&allowance());
        assert!(p.starts_with("(version 1)\n(deny default)\n"), "{p}");
        assert!(
            p.contains("(allow file-read* (literal \"/\") (subpath \"/usr\") (subpath \"/opt/my \\\"tools\\\"\"))"),
            "{p}"
        );
        assert!(
            p.contains("(allow file-read* (literal \"/repo/docs/a.md\"))"),
            "{p}"
        );
        assert!(
            p.contains(
                "(allow file-read-data (require-all (subpath \"/repo\") (vnode-type DIRECTORY)))"
            ),
            "{p}"
        );
        assert!(
            p.contains("(allow file-read* file-write* (subpath \"/tmp/sb\"))"),
            "{p}"
        );
        assert!(
            !p.contains("network"),
            "no network rule: deny default holds"
        );
        assert!(
            !p.contains("mach-lookup"),
            "no Mach service: deny default holds"
        );
    }

    #[test]
    fn a_tree_that_holds_the_repository_is_opened_only_around_it() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path().canonicalize().unwrap();
        let repo = top.join("src/app");
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        std::fs::create_dir_all(top.join("bin")).unwrap();
        std::fs::create_dir_all(top.join("src/other")).unwrap();
        std::fs::write(top.join("src/note"), "").unwrap();
        std::os::unix::fs::symlink(&repo, top.join("into")).unwrap();
        std::os::unix::fs::symlink(&top, top.join("src/up")).unwrap();
        let mut out = Vec::new();
        around(&top, &repo, &mut out);
        out.sort();
        assert_eq!(
            out,
            [top.join("bin"), top.join("src/note"), top.join("src/other")]
        );
    }

    #[test]
    fn the_allowance_leaves_path_directories_inside_the_repository_out() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(r.join(".git")).unwrap();
        std::fs::create_dir_all(r.join("bin")).unwrap();
        std::fs::write(r.join("a.md"), "").unwrap();
        let ctx = Ctx::for_template(&r.join("DOC.md"));
        let files = BTreeMap::from([(b"a.md".to_vec(), r.join("a.md"))]);
        let a = Allowance::new(&ctx, &files, Path::new("/tmp")).unwrap();
        assert_eq!(a.listing, r);
        assert_eq!(a.files, [r.join("a.md")]);
        assert!(a.system.iter().all(|p| !p.starts_with(&r)));
        assert!(a.system.contains(&PathBuf::from("/usr")));
    }
}
