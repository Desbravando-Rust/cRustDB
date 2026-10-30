//! Risk classification of raw Redis commands.

use crustdb_core::driver::SessionPurpose;
use crustdb_core::safety::Verdict;
use crustdb_core::store::Refused;

/// `DEL`/`UNLINK` with more keys than this asks for confirmation.
const DEL_CONFIRM_ABOVE: usize = 10;

/// Commands that run without asking: common reads and writes.
const RUN: &[&str] = &[
    "GET",
    "MGET",
    "STRLEN",
    "GETRANGE",
    "EXISTS",
    "TYPE",
    "TTL",
    "PTTL",
    "EXPIRETIME",
    "PEXPIRETIME",
    "SCAN",
    "SSCAN",
    "HSCAN",
    "ZSCAN",
    "DBSIZE",
    "PING",
    "ECHO",
    "TIME",
    "INFO",
    "RANDOMKEY",
    "OBJECT",
    "MEMORY",
    "HGET",
    "HMGET",
    "HGETALL",
    "HKEYS",
    "HVALS",
    "HLEN",
    "HEXISTS",
    "LRANGE",
    "LLEN",
    "LINDEX",
    "SMEMBERS",
    "SCARD",
    "SISMEMBER",
    "SRANDMEMBER",
    "ZRANGE",
    "ZRANGEBYSCORE",
    "ZREVRANGE",
    "ZCARD",
    "ZSCORE",
    "ZRANK",
    "ZCOUNT",
    "XRANGE",
    "XREVRANGE",
    "XLEN",
    "XINFO",
    "SET",
    "SETEX",
    "SETNX",
    "MSET",
    "APPEND",
    "INCR",
    "INCRBY",
    "DECR",
    "DECRBY",
    "EXPIRE",
    "PEXPIRE",
    "EXPIREAT",
    "PERSIST",
    "RENAME",
    "HSET",
    "HSETNX",
    "HMSET",
    "HDEL",
    "HINCRBY",
    "LPUSH",
    "RPUSH",
    "LPOP",
    "RPOP",
    "LSET",
    "LREM",
    "SADD",
    "SREM",
    "SPOP",
    "ZADD",
    "ZREM",
    "ZINCRBY",
    "XADD",
    "XDEL",
    "XTRIM",
];

const PASSWORD: &[&str] = &[
    "FLUSHALL",
    "FLUSHDB",
    "SHUTDOWN",
    "EVAL",
    "EVALSHA",
    "EVAL_RO",
    "EVALSHA_RO",
    "FCALL",
    "FCALL_RO",
];

/// Blocking, transactional, pub/sub or connection-state commands cannot work on a shared
/// multiplexed connection.
const REFUSED: &[&str] = &[
    "MULTI",
    "EXEC",
    "DISCARD",
    "WATCH",
    "UNWATCH",
    "SUBSCRIBE",
    "PSUBSCRIBE",
    "SSUBSCRIBE",
    "MONITOR",
    "BLPOP",
    "BRPOP",
    "BLMOVE",
    "BRPOPLPUSH",
    "BZPOPMIN",
    "BZPOPMAX",
    "BLMPOP",
    "BZMPOP",
    "HELLO",
    "RESET",
    "QUIT",
    "SELECT",
    "AUTH",
];

const CONFIRM: &[&str] = &[
    "DEBUG",
    "KEYS",
    "MIGRATE",
    "REPLICAOF",
    "SLAVEOF",
    "MODULE",
    "SAVE",
    "SWAPDB",
];

/// Subcommands that need confirmation, by command.
const CONFIRM_SUB: &[(&str, &[&str])] = &[
    ("CONFIG", &["SET", "REWRITE", "RESETSTAT"]),
    ("SCRIPT", &["FLUSH", "LOAD"]),
    ("FUNCTION", &["FLUSH", "DELETE", "LOAD"]),
    ("CLIENT", &["KILL"]),
    ("ACL", &["SETUSER", "DELUSER"]),
];

/// Sub-commands that run; any other subcommand of a command listed here is unknown.
const RUN_SUB: &[(&str, &[&str])] = &[
    ("CONFIG", &["GET"]),
    ("SCRIPT", &["EXISTS"]),
    ("FUNCTION", &["LIST", "STATS"]),
    ("CLIENT", &["LIST", "ID", "INFO", "GETNAME"]),
    ("ACL", &["WHOAMI", "LIST", "USERS", "GETUSER", "CAT"]),
];

fn upper(b: &[u8]) -> String {
    String::from_utf8_lossy(&b.to_ascii_uppercase()).into_owned()
}

fn refuse<T>(reason: impl Into<String>) -> Result<T, Refused> {
    Err(Refused {
        reason: reason.into(),
    })
}

fn has_block(args: &[Vec<u8>]) -> bool {
    args.iter().any(|a| a.eq_ignore_ascii_case(b"BLOCK"))
}

/// Classifies a command; `Err` means it must not run at all.
pub fn classify(argv: &[Vec<u8>]) -> Result<(Verdict, String), Refused> {
    let Some(first) = argv.first() else {
        return refuse("empty command");
    };
    let name = upper(first);
    let sub = argv.get(1).map(|s| upper(s)).unwrap_or_default();
    let n = name.as_str();
    let ok = |v, why: &str| Ok((v, why.to_owned()));

    if REFUSED.contains(&n) {
        return refuse(format!("{n} is not supported in this client"));
    }
    if matches!(n, "XREAD" | "XREADGROUP") {
        return if has_block(&argv[1..]) {
            refuse(format!("blocking {n} is not supported in this client"))
        } else {
            ok(Verdict::Confirm, "unrecognised command")
        };
    }
    if n == "CLIENT" && sub == "REPLY" {
        return refuse("CLIENT REPLY would desynchronise the connection");
    }
    if PASSWORD.contains(&n) {
        return ok(
            Verdict::Password,
            &format!("{n} can wipe data or run arbitrary scripts"),
        );
    }
    if matches!(n, "DEL" | "UNLINK") {
        return if argv.len() - 1 > DEL_CONFIRM_ABOVE {
            ok(
                Verdict::Confirm,
                &format!("{n} of more than {DEL_CONFIRM_ABOVE} keys"),
            )
        } else {
            ok(Verdict::Run, "")
        };
    }
    if CONFIRM.contains(&n) {
        return ok(Verdict::Confirm, &format!("{n} is a risky command"));
    }
    if let Some((_, subs)) = CONFIRM_SUB.iter().find(|(c, _)| *c == n)
        && subs.contains(&sub.as_str())
    {
        return ok(Verdict::Confirm, &format!("{n} {sub} changes server state"));
    }
    if let Some((_, subs)) = RUN_SUB.iter().find(|(c, _)| *c == n) {
        return if subs.contains(&sub.as_str()) {
            ok(Verdict::Run, "")
        } else {
            ok(Verdict::Confirm, "unrecognised command")
        };
    }
    if RUN.contains(&n) {
        return ok(Verdict::Run, "");
    }
    ok(Verdict::Confirm, "unrecognised command")
}

/// Only the editor may run arbitrary commands; refusals from `classify` still apply.
pub(crate) fn guard_run(purpose: SessionPurpose, argv: &[Vec<u8>]) -> Result<(), Refused> {
    if purpose != SessionPurpose::Editor {
        return refuse("only editor sessions can run commands");
    }
    classify(argv).map(|_| ())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<Vec<u8>> {
        parts.iter().map(|p| p.as_bytes().to_vec()).collect()
    }

    fn verdict(parts: &[&str]) -> Verdict {
        classify(&argv(parts)).unwrap().0
    }

    #[test]
    fn flushall_requires_password() {
        // Act & Assert
        for cmd in ["FLUSHALL", "FLUSHDB", "SHUTDOWN"] {
            assert_eq!(verdict(&[cmd]), Verdict::Password, "{cmd}");
        }
    }

    #[test]
    fn config_get_runs_but_config_set_confirms() {
        // Act & Assert
        assert_eq!(verdict(&["CONFIG", "GET", "databases"]), Verdict::Run);
        for sub in ["SET", "REWRITE", "RESETSTAT"] {
            assert_eq!(verdict(&["CONFIG", sub]), Verdict::Confirm, "{sub}");
        }
    }

    #[test]
    fn del_over_key_limit_confirms() {
        // Arrange
        let mut few = vec!["DEL"];
        few.extend(["k"; DEL_CONFIRM_ABOVE]);
        let mut many = vec!["UNLINK"];
        many.extend(["k"; DEL_CONFIRM_ABOVE + 1]);

        // Act & Assert
        assert_eq!(verdict(&few), Verdict::Run);
        assert_eq!(verdict(&many), Verdict::Confirm);
    }

    #[test]
    fn unknown_command_confirms() {
        // Act & Assert
        assert_eq!(verdict(&["FROBNICATE", "x"]), Verdict::Confirm);
        assert_eq!(verdict(&["KEYS", "*"]), Verdict::Confirm);
    }

    #[test]
    fn command_name_is_case_insensitive() {
        // Act & Assert
        assert_eq!(verdict(&["flushall"]), Verdict::Password);
        assert_eq!(verdict(&["Config", "set", "x", "y"]), Verdict::Confirm);
        assert_eq!(verdict(&["gEt", "k"]), Verdict::Run);
        assert!(classify(&argv(&["multi"])).is_err());
    }

    #[test]
    fn subscribe_monitor_multi_rejected() {
        // Act & Assert
        for cmd in [
            "MULTI",
            "EXEC",
            "WATCH",
            "SUBSCRIBE",
            "PSUBSCRIBE",
            "SSUBSCRIBE",
            "MONITOR",
            "BLPOP",
            "BRPOP",
            "BLMOVE",
            "BRPOPLPUSH",
            "BZPOPMIN",
            "BZPOPMAX",
            "BLMPOP",
            "BZMPOP",
        ] {
            assert!(classify(&argv(&[cmd, "k"])).is_err(), "{cmd}");
        }
        assert!(classify(&[]).is_err());
    }

    #[test]
    fn eval_cannot_bypass_flushall_password() {
        // Act & Assert
        for cmd in [
            "EVAL",
            "EVALSHA",
            "EVAL_RO",
            "EVALSHA_RO",
            "FCALL",
            "FCALL_RO",
        ] {
            assert_eq!(verdict(&[cmd, "x", "0"]), Verdict::Password, "{cmd}");
        }
    }

    #[test]
    fn script_load_and_function_load_confirm() {
        // Act & Assert
        for cmd in [
            &["SCRIPT", "LOAD", "return 1"][..],
            &["SCRIPT", "FLUSH"],
            &["FUNCTION", "LOAD", "code"],
            &["FUNCTION", "FLUSH"],
            &["FUNCTION", "DELETE", "lib"],
        ] {
            assert_eq!(verdict(cmd), Verdict::Confirm, "{cmd:?}");
        }
    }

    #[test]
    fn connection_state_commands_rejected() {
        // Act & Assert
        for cmd in [
            &["CLIENT", "REPLY", "OFF"][..],
            &["HELLO", "3"],
            &["RESET"],
            &["QUIT"],
            &["SELECT", "1"],
            &["AUTH", "pw"],
        ] {
            assert!(classify(&argv(cmd)).is_err(), "{cmd:?}");
        }
        assert_eq!(verdict(&["CLIENT", "KILL", "id", "1"]), Verdict::Confirm);
    }

    #[test]
    fn xread_block_rejected() {
        // Act & Assert
        assert!(classify(&argv(&["XREAD", "block", "0", "STREAMS", "s", "$"])).is_err());
        assert!(
            classify(&argv(&[
                "XREADGROUP",
                "GROUP",
                "g",
                "c",
                "BLOCK",
                "5",
                "STREAMS",
                "s",
                ">"
            ]))
            .is_err()
        );
        assert_eq!(verdict(&["XREAD", "STREAMS", "s", "0"]), Verdict::Confirm);
    }

    #[test]
    fn non_editor_purpose_refuses_run() {
        // Arrange
        let get = argv(&["GET", "k"]);

        // Act & Assert
        for p in [
            SessionPurpose::Metadata,
            SessionPurpose::Data,
            SessionPurpose::Export,
        ] {
            assert!(guard_run(p, &get).is_err(), "{p:?}");
        }
        assert!(guard_run(SessionPurpose::Editor, &get).is_ok());
        assert!(guard_run(SessionPurpose::Editor, &argv(&["MULTI"])).is_err());
    }
}
