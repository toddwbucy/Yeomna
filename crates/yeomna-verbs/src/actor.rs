//! Who is calling, per V3: the kernel's answer and never the caller's.
//!
//! In embedded mode there is no socket peer to ask, so the uid comes
//! from `/proc/self/status`, which the kernel writes and the process
//! cannot forge from inside. `USER` and `LOGNAME` are the caller's to
//! set and are not consulted. `/etc/passwd` is a naming table: it turns
//! the uid into something an audit row reads well, and when it cannot,
//! the uid itself is the actor.

use std::path::Path;

/// The real uid of this process, from the kernel.
fn real_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    // "Uid:\treal\teffective\tsaved\tfilesystem". The real uid is who
    // started this, which is the one an audit row should name.
    status
        .lines()
        .find_map(|l| l.strip_prefix("Uid:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// The login name for a uid, from the passwd table.
fn name_for(uid: u32, passwd: &Path) -> Option<String> {
    let text = std::fs::read_to_string(passwd).ok()?;
    name_in_passwd(uid, &text)
}

/// Split out so the table can be tested without one on disk.
fn name_in_passwd(uid: u32, text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password = fields.next()?;
        let entry: u32 = fields.next()?.parse().ok()?;
        (entry == uid && !name.is_empty()).then(|| name.to_string())
    })
}

/// The login name for a uid, or `None` when this machine cannot name
/// it. The daemon asks this about a socket peer and refuses when the
/// answer is `None` (D6), because an audit row that cannot say who
/// acted is not a record and a synthetic name would make one.
pub fn name_for_uid(uid: u32) -> Option<String> {
    name_for(uid, Path::new("/etc/passwd"))
}

/// The actor this process calls as. A name when the table has one,
/// `uid:<n>` when it does not, and `unknown` when even the kernel would
/// not say, which is a machine shaped in a way this appliance has not
/// met and is recorded rather than guessed at.
pub fn from_kernel() -> String {
    match real_uid() {
        Some(uid) => name_for_uid(uid).unwrap_or_else(|| format!("uid:{uid}")),
        None => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\n\
                          bin:x:1:1::/:/usr/bin/nologin\n\
                          todd:x:1000:1000::/home/todd:/bin/bash\n";

    #[test]
    fn the_table_names_the_uid_it_carries() {
        assert_eq!(name_in_passwd(1000, PASSWD).as_deref(), Some("todd"));
        assert_eq!(name_in_passwd(0, PASSWD).as_deref(), Some("root"));
        assert_eq!(name_in_passwd(65534, PASSWD), None, "absent stays absent");
    }

    #[test]
    fn a_malformed_table_yields_no_name_rather_than_a_wrong_one() {
        assert_eq!(name_in_passwd(1000, "garbage\n:::\n"), None);
        assert_eq!(
            name_in_passwd(1000, ":x:1000:1000::/:/bin/sh\n"),
            None,
            "an empty name is not a name"
        );
    }

    /// The kernel answers on this platform, and the answer is not the
    /// environment's to change (FR5).
    #[test]
    fn the_actor_comes_from_the_kernel_not_the_environment() {
        let actor = from_kernel();
        assert!(!actor.is_empty());
        assert_ne!(actor, "unknown", "/proc/self/status is readable here");
        // Whatever USER says, the uid is what the kernel wrote.
        let uid = real_uid().expect("a uid on this platform");
        let expected =
            name_for(uid, Path::new("/etc/passwd")).unwrap_or_else(|| format!("uid:{uid}"));
        assert_eq!(actor, expected);
    }
}
