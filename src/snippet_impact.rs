//! Static, execution-free blast-radius analysis of a snippet command, surfaced
//! by the Snippets detail IMPACT card. A snippet is a single-line shell command
//! run over SSH against a fleet of hosts; this judges what it does BEFORE a
//! fleet run, so an operator sees the blast radius at a glance.
//!
//! Pipeline (the ShellCheck/explainshell model, hand-rolled with no parser
//! crate): quote-aware tokenize -> split the line into per-command segments on
//! shell operators (`| ; & && || ( )`) -> resolve each segment's command head
//! (skip `VAR=val` assignments, unwrap `sudo`/`env`/`nice`/`timeout`/`xargs`/...
//! wrappers) -> classify the head + its flags/sub-command via a curated
//! capability table, plus structural signals (redirects, pipe-to-interpreter,
//! command substitution). Quote-awareness means `echo "rm -rf"` and `grep '>'`
//! are inert, which the old flat tokenizer got wrong.
//!
//! This is ADVISORY, not a sandbox: substitution recurses one level, exotic
//! syntax (process substitution, here-docs, ANSI-C quoting) is best-effort, and
//! an unknown command head yields an explicit "effect unclear" finding so a
//! green "read-only" verdict is never emitted falsely.

/// Worst-to-best severity of an impact finding. `Ord`-derived so the card
/// verdict is simply `findings.map(severity).max()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// No state change observed: read-only, safe to fan out.
    ReadOnly,
    /// Changes state on each host, but recoverably.
    WritesState,
    /// Runs as root, fetches and runs remote code, or stops services.
    Elevated,
    /// Irreversible data loss or a fleet-wide outage.
    Critical,
}

/// What kind of effect a finding describes. Drives the callout wording in the
/// UI (the prose lives in `messages`, not here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Recoverable destructive change (delete, overwrite, recursive perms).
    Destructive,
    /// Unrecoverable data loss (rm of a system path, dd/mkfs of a device,
    /// `git reset --hard`, `find -delete`).
    Irreversible,
    /// Runs with elevated privilege (`sudo`/`su`/`doas`).
    Privilege,
    /// Fetches code from the network and runs it (`curl ... | sh`).
    RemoteExec,
    /// Stops, restarts or kills services/processes/containers.
    Service,
    /// Reboots or powers off the host (availability impact).
    Availability,
    /// Installs/removes/upgrades system packages.
    Package,
    /// Truncates or overwrites a file via a `>` redirect.
    Redirect,
    /// Reads a path that commonly holds secrets (keys, .env, shadow).
    Secrets,
    /// Command head is not in the capability table: effect unverified.
    Unknown,
}

/// One impact signal derived from the command. `subject` is the specific
/// trigger (verb, flag, target or path) the UI parameterises its wording with;
/// it is NOT user-facing prose (that is built in `messages::snippet`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub category: Category,
    pub subject: String,
}

impl Finding {
    fn new(severity: Severity, category: Category, subject: impl Into<String>) -> Self {
        Self {
            severity,
            category,
            subject: subject.into(),
        }
    }
}

/// The full impact analysis of a command: an ordered, de-duplicated set of
/// findings. No findings means the command is read-only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandImpact {
    pub findings: Vec<Finding>,
}

impl CommandImpact {
    /// The headline verdict: the worst severity across all findings, or
    /// [`Severity::ReadOnly`] when there are none.
    pub fn verdict(&self) -> Severity {
        self.findings
            .iter()
            .map(|f| f.severity)
            .max()
            .unwrap_or(Severity::ReadOnly)
    }

    /// Findings worst-severity first (stable within a severity), for the card's
    /// callout list. The caller caps the count.
    pub fn callouts(&self) -> Vec<&Finding> {
        let mut v: Vec<&Finding> = self.findings.iter().collect();
        v.sort_by_key(|f| std::cmp::Reverse(f.severity));
        v
    }

    /// True when the command runs with elevated privilege.
    pub fn uses_sudo(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.category == Category::Privilege)
    }

    /// True when the command needs a careful read before a fleet run: anything
    /// above a plain state write.
    pub fn is_elevated_or_destructive(&self) -> bool {
        self.verdict() >= Severity::Elevated
    }
}

// =========================================================================
// Capability table: which command heads / flags / sub-commands escalate.
// =========================================================================

/// Command heads with no state-changing effect. A head in this set produces no
/// finding; a head NOT here and not in the rule table is `Unknown` (never a
/// silent all-clear). Kept deliberately conservative.
const READ_ONLY_HEADS: &[&str] = &[
    "ls",
    "ll",
    "pwd",
    "cd",
    "echo",
    "printf",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "ag",
    "awk",
    "cut",
    "sort",
    "uniq",
    "wc",
    "tr",
    "column",
    "true",
    "false",
    "test",
    "stat",
    "file",
    "locate",
    "which",
    "type",
    "whereis",
    "readlink",
    "realpath",
    "basename",
    "dirname",
    "du",
    "df",
    "free",
    "uptime",
    "uname",
    "hostname",
    "hostnamectl",
    "whoami",
    "id",
    "groups",
    "w",
    "who",
    "last",
    "date",
    "cal",
    "printenv",
    "set",
    "ps",
    "pgrep",
    "pstree",
    "top",
    "htop",
    "vmstat",
    "iostat",
    "mpstat",
    "lscpu",
    "lsblk",
    "lsusb",
    "lspci",
    "lsof",
    "ip",
    "ifconfig",
    "ss",
    "netstat",
    "ping",
    "ping6",
    "traceroute",
    "dig",
    "nslookup",
    "host",
    "getent",
    "curl",
    "wget",
    "nc",
    "telnet",
    "openssl",
    "md5sum",
    "sha256sum",
    "sha1sum",
    "cksum",
    "sensors",
    "dmesg",
    "journalctl",
    // Read / pipe-common transformers and dumpers (their output goes via a
    // redirect, which is classified separately).
    "pg_dump",
    "pg_dumpall",
    "mysqldump",
    "gzip",
    "gunzip",
    "zcat",
    "gzcat",
    "xz",
    "bzip2",
    "base64",
    "jq",
    "yq",
    "nl",
    "tac",
];

/// Wrapper commands that run another command; the real head is what follows.
/// `sudo`/`su`/`doas` additionally flag a privilege finding for the segment.
const WRAPPERS: &[&str] = &[
    "sudo", "doas", "su", "env", "nice", "ionice", "nohup", "timeout", "stdbuf", "setsid", "time",
    "command", "builtin", "exec", "xargs",
];

/// Interpreters that run code from stdin (the downstream side of `curl | sh`).
const INTERPRETERS: &[&str] = &[
    "sh", "bash", "dash", "zsh", "ksh", "fish", "python", "python2", "python3", "perl", "ruby",
    "node", "php", "lua", "tclsh",
];

/// Commands that fetch from the network (the upstream side of `curl | sh`).
const FETCHERS: &[&str] = &["curl", "wget", "fetch", "http", "https"];

/// Specific path markers for files that commonly hold secrets. Deliberately
/// narrow (no loose "secret"/"token"/"private" substrings) so an ordinary log
/// like `/var/log/private-api.log` is not mistaken for a key store.
const SECRET_PATTERNS: &[&str] = &[
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    "id_dsa",
    ".pem",
    ".key",
    ".pfx",
    ".p12",
    "/etc/shadow",
    "/etc/gshadow",
    "/.ssh/",
    "authorized_keys",
    ".env",
    ".aws/credentials",
    ".npmrc",
    ".netrc",
];

/// System paths whose recursive deletion is unrecoverable host damage.
const SYSTEM_PATHS: &[&str] = &[
    "/", "/*", "/bin", "/sbin", "/usr", "/etc", "/var", "/lib", "/lib64", "/boot", "/opt", "/root",
    "/home", "/srv", "/dev", "/proc", "/sys",
];

// =========================================================================
// Quote-aware segmenter
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RedirKind {
    /// `>` truncate / clobber.
    Truncate,
    /// `>>` append.
    Append,
    /// `<`, `2>`, `&>`, `2>&1`, here-docs: input or fd plumbing (benign).
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Redirect {
    kind: RedirKind,
    target: String,
}

/// One command in the pipeline: quote-stripped words, redirects, any inner
/// command substitutions, and whether it follows a `|`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Segment {
    words: Vec<String>,
    redirects: Vec<Redirect>,
    substitutions: Vec<String>,
    after_pipe: bool,
}

/// Capture a balanced `( ... )` group. `start` is the index of the opening `(`.
/// Returns the inner text and the index just past the matching `)`.
fn capture_balanced(chars: &[char], start: usize) -> (String, usize) {
    let mut depth = 0i32;
    let mut inner = String::new();
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '(' => {
                depth += 1;
                if depth == 1 {
                    i += 1;
                    continue;
                }
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (inner, i + 1);
                }
            }
            _ => {}
        }
        inner.push(chars[i]);
        i += 1;
    }
    (inner, i)
}

/// Capture a backtick `` `...` `` substitution. `start` is the opening backtick.
fn capture_backtick(chars: &[char], start: usize) -> (String, usize) {
    let mut inner = String::new();
    let mut i = start + 1;
    while i < chars.len() && chars[i] != '`' {
        inner.push(chars[i]);
        i += 1;
    }
    (inner, (i + 1).min(chars.len()))
}

/// Classify a redirect operator at `chars[i]` (`>` or `<`). Returns the kind and
/// how many chars the operator spans.
fn redirect_kind(chars: &[char], i: usize) -> (RedirKind, usize) {
    let next = chars.get(i + 1).copied();
    if chars[i] == '>' {
        match next {
            Some('>') => (RedirKind::Append, 2),
            Some('|') => (RedirKind::Truncate, 2),
            Some('&') => (RedirKind::Other, 2), // >& fd dup
            _ => (RedirKind::Truncate, 1),
        }
    } else {
        match next {
            Some('<') | Some('&') => (RedirKind::Other, 2),
            _ => (RedirKind::Other, 1),
        }
    }
}

/// Read a redirect target word starting at `i`, handling quotes and `$(...)`
/// so a timestamped path like `/backups/x-$(date +%F).gz` is one target, not a
/// spurious subshell split. Returns the target text and the index past it.
fn read_target(chars: &[char], mut i: usize) -> (String, usize) {
    let n = chars.len();
    let mut target = String::new();
    while i < n && !chars[i].is_whitespace() && !matches!(chars[i], '|' | '&' | ';' | '>' | '<') {
        match chars[i] {
            '\'' | '"' => {
                let q = chars[i];
                i += 1;
                while i < n && chars[i] != q {
                    target.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '$' if chars.get(i + 1) == Some(&'(') => {
                let (inner, ni) = capture_balanced(chars, i + 1);
                target.push_str("$(");
                target.push_str(&inner);
                target.push(')');
                i = ni;
            }
            '(' | ')' => break,
            c => {
                target.push(c);
                i += 1;
            }
        }
    }
    (target, i)
}

/// Quote-aware split of a single-line command into per-command segments. Only
/// UNQUOTED operators split or redirect; quoted text and quoted operators are
/// inert, which is what kills the old tokenizer's `echo "rm -rf"` / `grep '>'`
/// false positives.
fn segment(command: &str) -> Vec<Segment> {
    let chars: Vec<char> = command.chars().collect();
    let n = chars.len();
    let mut segments: Vec<Segment> = Vec::new();
    let mut cur = Segment::default();
    let mut word = String::new();
    let mut next_after_pipe = false;
    let mut i = 0usize;

    macro_rules! flush_word {
        () => {
            if !word.is_empty() {
                cur.words.push(std::mem::take(&mut word));
            }
        };
    }
    macro_rules! flush_segment {
        ($after:expr) => {{
            flush_word!();
            cur.after_pipe = next_after_pipe;
            if !cur.words.is_empty() || !cur.redirects.is_empty() || !cur.substitutions.is_empty() {
                segments.push(std::mem::take(&mut cur));
            } else {
                cur = Segment::default();
            }
            next_after_pipe = $after;
        }};
    }

    while i < n {
        let c = chars[i];
        match c {
            '\\' => {
                if i + 1 < n {
                    word.push(chars[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            '\'' => {
                i += 1;
                while i < n && chars[i] != '\'' {
                    word.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                i += 1;
                while i < n && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < n {
                        word.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    if chars[i] == '$' && i + 1 < n && chars[i + 1] == '(' {
                        let (inner, ni) = capture_balanced(&chars, i + 1);
                        cur.substitutions.push(inner);
                        i = ni;
                        continue;
                    }
                    if chars[i] == '`' {
                        let (inner, ni) = capture_backtick(&chars, i);
                        cur.substitutions.push(inner);
                        i = ni;
                        continue;
                    }
                    word.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '$' if i + 1 < n && chars[i + 1] == '(' => {
                let (inner, ni) = capture_balanced(&chars, i + 1);
                cur.substitutions.push(inner);
                i = ni;
            }
            '`' => {
                let (inner, ni) = capture_backtick(&chars, i);
                cur.substitutions.push(inner);
                i = ni;
            }
            c if c.is_whitespace() => {
                flush_word!();
                i += 1;
            }
            '|' => {
                if chars.get(i + 1) == Some(&'|') {
                    flush_segment!(false);
                    i += 2;
                } else {
                    flush_segment!(true);
                    i += 1;
                }
            }
            '&' => {
                if chars.get(i + 1) == Some(&'&') {
                    flush_segment!(false);
                    i += 2;
                } else if chars.get(i + 1) == Some(&'>') {
                    // `&>` / `&>>`: redirect stdout+stderr (benign for impact).
                    flush_word!();
                    let span = if chars.get(i + 2) == Some(&'>') { 3 } else { 2 };
                    i += span;
                    while i < n && chars[i].is_whitespace() {
                        i += 1;
                    }
                    let (target, ni) = read_target(&chars, i);
                    i = ni;
                    cur.redirects.push(Redirect {
                        kind: RedirKind::Other,
                        target,
                    });
                } else {
                    flush_segment!(false);
                    i += 1;
                }
            }
            ';' => {
                flush_segment!(false);
                i += 1;
            }
            '(' | ')' => {
                flush_segment!(false);
                i += 1;
            }
            '>' | '<' => {
                // A lone leading fd digit (the `2` in `2>`) is part of the
                // operator, not an operand.
                if !word.is_empty() && word.chars().all(|c| c.is_ascii_digit()) {
                    word.clear();
                } else {
                    flush_word!();
                }
                let (kind, span) = redirect_kind(&chars, i);
                i += span;
                while i < n && chars[i].is_whitespace() {
                    i += 1;
                }
                let (target, ni) = read_target(&chars, i);
                i = ni;
                cur.redirects.push(Redirect { kind, target });
            }
            _ => {
                word.push(c);
                i += 1;
            }
        }
    }
    // Final flush of the trailing segment (no operator follows it).
    if !word.is_empty() {
        cur.words.push(word);
    }
    cur.after_pipe = next_after_pipe;
    if !cur.words.is_empty() || !cur.redirects.is_empty() || !cur.substitutions.is_empty() {
        segments.push(cur);
    }
    segments
}

// =========================================================================
// Classifier: segments -> findings
// =========================================================================

/// Basename of a command head (`/usr/bin/rm` -> `rm`).
fn basename(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// True for a `VAR=val` leading assignment (identifier before `=`, no `/`).
fn is_assignment(w: &str) -> bool {
    match w.split_once('=') {
        Some((k, _)) => {
            !k.is_empty()
                && !k.contains('/')
                && k.chars().all(|c| c.is_alphanumeric() || c == '_')
                && k.chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
        }
        None => false,
    }
}

/// How many of a wrapper's own option arguments to skip before its target
/// command (best-effort).
fn skip_wrapper_args(wrapper: &str, rest: &[String]) -> usize {
    let mut k = 0;
    match wrapper {
        "env" => {
            while k < rest.len() && (is_assignment(&rest[k]) || rest[k].starts_with('-')) {
                k += 1;
            }
        }
        "timeout" => {
            while k < rest.len() && rest[k].starts_with('-') {
                k += 1;
            }
            if k < rest.len() {
                k += 1; // the duration
            }
        }
        "nice" | "ionice" => {
            while k < rest.len() && rest[k].starts_with('-') {
                let takes_val = matches!(rest[k].as_str(), "-n" | "-c" | "-p");
                k += 1;
                if takes_val && k < rest.len() {
                    k += 1;
                }
            }
        }
        "sudo" | "doas" => {
            while k < rest.len() && rest[k].starts_with('-') {
                let takes_val = matches!(
                    rest[k].as_str(),
                    "-u" | "-g" | "-p" | "-C" | "-h" | "-r" | "-t" | "-U"
                );
                k += 1;
                if takes_val && k < rest.len() {
                    k += 1;
                }
            }
        }
        _ => {
            while k < rest.len() && rest[k].starts_with('-') {
                k += 1;
            }
        }
    }
    k
}

/// Resolve a segment's real command head: skip leading `VAR=val`, unwrap
/// wrappers. Returns `(head, args, privileged)` where `privileged` is set when a
/// `sudo`/`su`/`doas` wrapper was unwrapped.
fn resolve_head(seg: &Segment) -> Option<(String, Vec<String>, bool)> {
    let words = &seg.words;
    let mut idx = 0;
    let mut privileged = false;
    loop {
        while idx < words.len() && is_assignment(&words[idx]) {
            idx += 1;
        }
        let w = words.get(idx)?;
        if WRAPPERS.contains(&w.as_str()) {
            if matches!(w.as_str(), "sudo" | "su" | "doas") {
                privileged = true;
            }
            idx += 1;
            idx += skip_wrapper_args(w, words.get(idx..).unwrap_or(&[]));
            continue;
        }
        if w.starts_with('-') {
            // First token is a flag (e.g. the real command was a substitution):
            // no resolvable head.
            return None;
        }
        let args = words.get(idx + 1..).unwrap_or(&[]).to_vec();
        return Some((w.clone(), args, privileged));
    }
}

/// Non-flag positional arguments.
fn positionals(args: &[String]) -> Vec<&String> {
    args.iter().filter(|a| !a.starts_with('-')).collect()
}

/// First positional argument (the sub-command for git/docker/systemctl/...).
fn first_subcommand(args: &[String]) -> Option<&str> {
    args.iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
}

/// True when any short-flag bundle contains one of `wanted`, or a matching long
/// flag is present.
fn has_flag(args: &[String], wanted: &[char], long: &[&str]) -> bool {
    args.iter().any(|a| {
        (a.starts_with('-')
            && !a.starts_with("--")
            && a.chars().skip(1).any(|c| wanted.contains(&c)))
            || long.contains(&a.as_str())
    })
}

fn is_system_path(p: &str) -> bool {
    if p == "/" || p == "/*" {
        return true;
    }
    let t = p.trim_end_matches('/');
    SYSTEM_PATHS.contains(&t) || p.contains("/*")
}

fn looks_unbounded(p: &str) -> bool {
    p.contains('*') || p.contains('?') || p.starts_with('$') || p.contains("/$")
}

fn is_secret_path(p: &str) -> bool {
    let lower = p.to_lowercase();
    SECRET_PATTERNS.iter().any(|s| lower.contains(s))
}

/// A raw block device or any non-stdio `/dev` node: writing to it is destructive.
fn is_block_device(p: &str) -> bool {
    p.starts_with("/dev/") && !is_devnull(p)
}

/// System control file where ANY write (truncate or append) is high-impact:
/// editing it changes privilege, accounts, scheduling or boot/network config.
fn is_control_file(p: &str) -> bool {
    const CONTROL: &[&str] = &[
        "/etc/sudoers",
        "/etc/passwd",
        "/etc/shadow",
        "/etc/gshadow",
        "/etc/group",
        "/etc/crontab",
        "/etc/fstab",
        "/etc/hosts",
        "/etc/resolv.conf",
        "/etc/environment",
    ];
    CONTROL.contains(&p)
        || p.contains("authorized_keys")
        || p.starts_with("/etc/sudoers.d/")
        || p.starts_with("/etc/cron.")
        || p.starts_with("/etc/systemd/")
}

/// Absolute path at or under a system root: recursive deletion or a config write
/// here is unrecoverable host damage, not a local edit.
fn is_under_system_path(p: &str) -> bool {
    const ROOTS: &[&str] = &[
        "/etc/",
        "/var/lib/",
        "/boot/",
        "/usr/",
        "/lib/",
        "/lib64/",
        "/bin/",
        "/sbin/",
        "/root",
        "/sys/",
        "/proc/",
    ];
    is_system_path(p) || ROOTS.iter().any(|r| p.starts_with(r))
}

/// The user's home directory: `rm -rf ~` wipes everything on each host.
fn is_home(p: &str) -> bool {
    p == "~" || p == "$HOME" || p.starts_with("~/") || p.starts_with("$HOME/")
}

/// Classify all segments into findings (pipe-to-interpreter detection, then a
/// per-segment pass, plus one level of substitution recursion).
fn classify(segments: &[Segment], depth: usize) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Remote-fetch-and-execute: an interpreter head on a piped segment, when any
    // segment in the line fetches from the network.
    let heads: Vec<Option<(String, bool)>> = segments
        .iter()
        .map(|s| resolve_head(s).map(|(h, _, p)| (basename(&h).to_string(), p)))
        .collect();
    // Remote-fetch-and-execute: a fetcher anywhere, then an interpreter head in
    // a LATER segment. Covers both the piped form (`curl | sh`) and the two-step
    // form (`curl ... -o /tmp/x && bash /tmp/x`, `wget ...; sh ...`).
    let first_fetcher = heads.iter().position(|h| {
        h.as_ref()
            .is_some_and(|(n, _)| FETCHERS.contains(&n.as_str()))
    });
    if let Some(fi) = first_fetcher {
        for (idx, h) in heads.iter().enumerate() {
            if idx <= fi {
                continue;
            }
            if let Some((name, priv_)) = h {
                if INTERPRETERS.contains(&name.as_str()) {
                    let sev = if *priv_ {
                        Severity::Critical
                    } else {
                        Severity::Elevated
                    };
                    findings.push(Finding::new(sev, Category::RemoteExec, "curl|sh"));
                }
            }
        }
    }

    for seg in segments {
        classify_segment(seg, &mut findings, depth);
        if depth == 0 {
            for inner in &seg.substitutions {
                if !inner.trim().is_empty() {
                    findings.extend(classify(&segment(inner), depth + 1));
                }
            }
        }
    }
    findings
}

/// Per-segment classification: redirects, then the command head + flags.
fn classify_segment(seg: &Segment, findings: &mut Vec<Finding>, depth: usize) {
    for r in &seg.redirects {
        classify_redirect(r, findings);
    }
    if let Some((head, args, privileged)) = resolve_head(seg) {
        classify_head(basename(&head), &args, privileged, findings, depth);
    }
}

/// A redirect's impact: truncating a real file is a write; truncating a device
/// or writing (even appending) to a system control file is far worse.
fn classify_redirect(r: &Redirect, findings: &mut Vec<Finding>) {
    let t = &r.target;
    if t.is_empty() || is_devnull(t) {
        return;
    }
    match r.kind {
        RedirKind::Truncate => {
            if is_block_device(t) {
                findings.push(Finding::new(
                    Severity::Critical,
                    Category::Irreversible,
                    t.clone(),
                ));
            } else if is_control_file(t) || is_under_system_path(t) {
                findings.push(Finding::new(
                    Severity::Elevated,
                    Category::Redirect,
                    t.clone(),
                ));
            } else {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Redirect,
                    t.clone(),
                ));
            }
        }
        // Append is benign for ordinary files (a log line) but high-impact for a
        // control file (e.g. `>> /etc/sudoers`, `>> ~/.ssh/authorized_keys`).
        RedirKind::Append => {
            if is_control_file(t) {
                findings.push(Finding::new(
                    Severity::Elevated,
                    Category::Redirect,
                    t.clone(),
                ));
            }
        }
        RedirKind::Other => {}
    }
}

fn is_devnull(t: &str) -> bool {
    matches!(t, "/dev/null" | "/dev/stdout" | "/dev/stderr")
}

/// The curated capability table: maps a command head + its flags/sub-command to
/// impact findings. The heart of the analysis. Read-only forms (e.g. `git
/// status`, `systemctl status`) intentionally emit nothing; an unknown head
/// emits an `Unknown` finding so a green verdict is never falsely shown.
//
// One `match head` is the readable single source of truth for the rule table.
// Splitting it across helper functions to dodge the line cap would scatter the
// curated rules and make coverage harder to audit, so the allow is intentional.
#[allow(clippy::too_many_lines)]
fn classify_head(
    head: &str,
    args: &[String],
    privileged: bool,
    findings: &mut Vec<Finding>,
    depth: usize,
) {
    if privileged {
        findings.push(Finding::new(
            Severity::Elevated,
            Category::Privilege,
            "sudo",
        ));
    }

    // Filesystem-image utilities (mkfs, mkfs.ext4, ...).
    if head == "mkfs" || head.starts_with("mkfs.") {
        findings.push(Finding::new(
            Severity::Critical,
            Category::Irreversible,
            "mkfs",
        ));
        return;
    }

    match head {
        "rm" => {
            let recursive = has_flag(args, &['r', 'f', 'R'], &["--recursive", "--force"]);
            let ps = positionals(args);
            // Irreversible host damage: a top-level system root, the home dir, an
            // unbounded glob/variable target, or any recursive delete under a
            // system root (/etc/letsencrypt, /var/lib/mysql, ...).
            let critical = ps.iter().any(|p| {
                is_system_path(p)
                    || is_home(p)
                    || (recursive && (looks_unbounded(p) || is_under_system_path(p)))
            });
            // Recursive delete of an absolute location is worse than a local one.
            let absolute = ps.iter().any(|p| p.starts_with('/'));
            if critical {
                findings.push(Finding::new(
                    Severity::Critical,
                    Category::Irreversible,
                    "rm",
                ));
            } else if recursive && absolute {
                findings.push(Finding::new(
                    Severity::Elevated,
                    Category::Destructive,
                    "rm -rf",
                ));
            } else {
                let s = if recursive { "rm -rf" } else { "rm" };
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Destructive,
                    s,
                ));
            }
        }
        "rmdir" | "unlink" => {
            findings.push(Finding::new(
                Severity::WritesState,
                Category::Destructive,
                head,
            ));
        }
        "shred" | "wipefs" | "fdisk" | "sgdisk" | "parted" | "mkswap" => {
            findings.push(Finding::new(
                Severity::Critical,
                Category::Irreversible,
                head,
            ));
        }
        "dd" => {
            if let Some(of) = args.iter().find_map(|a| a.strip_prefix("of=")) {
                if of.starts_with("/dev/") {
                    findings.push(Finding::new(
                        Severity::Critical,
                        Category::Irreversible,
                        "dd",
                    ));
                } else {
                    findings.push(Finding::new(
                        Severity::WritesState,
                        Category::Destructive,
                        "dd",
                    ));
                }
            }
        }
        "truncate" => {
            findings.push(Finding::new(
                Severity::WritesState,
                Category::Destructive,
                "truncate",
            ));
        }
        "find" => {
            if args.iter().any(|a| a == "-delete") {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Destructive,
                    "find -delete",
                ));
            }
            // Classify the command that -exec actually runs (the token after
            // -exec, until `;`/`+`), not any "rm" anywhere in the args. So
            // `find -exec chmod 644 {} ;` is destructive but `find -name rm
            // -exec ls {} ;` is read-only.
            if let Some(pos) = args
                .iter()
                .position(|a| matches!(a.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir"))
            {
                let rest = args.get(pos + 1..).unwrap_or(&[]);
                let end = rest
                    .iter()
                    .position(|a| a == ";" || a == "+" || a == "\\;")
                    .unwrap_or(rest.len());
                if let Some(cmd) = rest.get(..end).and_then(<[String]>::first) {
                    classify_head(
                        basename(cmd),
                        rest.get(1..end).unwrap_or(&[]),
                        false,
                        findings,
                        depth,
                    );
                }
            }
        }
        "sed" => {
            if args.iter().any(|a| {
                a == "-i"
                    || a.starts_with("--in-place")
                    || (a.starts_with("-i") && a.len() > 2 && !a.starts_with("--"))
            }) {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Destructive,
                    "sed -i",
                ));
            }
        }
        "tee" => {
            // Writing to a system/control path (e.g. `... | sudo tee /etc/...`)
            // is far worse than teeing to a local file.
            if positionals(args)
                .iter()
                .any(|p| is_control_file(p) || is_under_system_path(p) || is_block_device(p))
            {
                findings.push(Finding::new(Severity::Elevated, Category::Redirect, "tee"));
            } else {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Redirect,
                    "tee",
                ));
            }
        }
        "mv" | "cp" | "install" | "ln" => {
            findings.push(Finding::new(
                Severity::WritesState,
                Category::Destructive,
                head,
            ));
        }
        "chmod" => {
            let recursive = has_flag(args, &['R'], &["--recursive"]);
            let world = positionals(args)
                .iter()
                .any(|p| p.contains("777") || p.contains("o+w") || p.contains("a+w"));
            let s = if world {
                "chmod 777"
            } else if recursive {
                "chmod -R"
            } else {
                "chmod"
            };
            findings.push(Finding::new(
                Severity::WritesState,
                Category::Destructive,
                s,
            ));
        }
        "chown" | "chgrp" => {
            let recursive = has_flag(args, &['R'], &["--recursive"]);
            let s = if recursive {
                format!("{head} -R")
            } else {
                head.to_string()
            };
            findings.push(Finding::new(
                Severity::WritesState,
                Category::Destructive,
                s,
            ));
        }
        "systemctl" => match first_subcommand(args) {
            Some("reboot") | Some("poweroff") | Some("halt") => {
                findings.push(Finding::new(
                    Severity::Critical,
                    Category::Availability,
                    "systemctl",
                ));
            }
            Some(s @ ("stop" | "restart" | "kill" | "disable" | "mask" | "isolate")) => {
                findings.push(Finding::new(
                    Severity::Elevated,
                    Category::Service,
                    format!("systemctl {s}"),
                ));
            }
            Some(s @ ("start" | "enable" | "reload" | "daemon-reload" | "set-default")) => {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Service,
                    format!("systemctl {s}"),
                ));
            }
            _ => {}
        },
        "service" => {
            if positionals(args)
                .iter()
                .any(|p| matches!(p.as_str(), "stop" | "restart" | "reload"))
            {
                findings.push(Finding::new(
                    Severity::Elevated,
                    Category::Service,
                    "service",
                ));
            }
        }
        "kill" | "pkill" | "killall" => {
            // `kill -0` (liveness probe) and `kill -l` (list signals) send no
            // signal: read-only.
            if !args.iter().any(|a| a == "-0" || a == "-l" || a == "-L") {
                findings.push(Finding::new(Severity::Elevated, Category::Service, head));
            }
        }
        "reboot" | "shutdown" | "halt" | "poweroff" | "init" => {
            findings.push(Finding::new(
                Severity::Critical,
                Category::Availability,
                head,
            ));
        }
        "docker" | "podman" => {
            // Resolve up to the two leading sub-commands so nested forms
            // (`docker compose down`, `docker container rm`, `docker system
            // prune`) are classified, not just top-level ones.
            let ps = positionals(args);
            let s1 = ps.first().map(|s| s.as_str());
            let s2 = ps.get(1).map(|s| s.as_str());
            match (s1, s2) {
                (Some("system"), Some("prune")) | (Some("volume"), Some("rm" | "prune")) => {
                    findings.push(Finding::new(
                        Severity::Elevated,
                        Category::Destructive,
                        format!("{head} {} prune", s1.unwrap_or("")),
                    ));
                }
                (Some("rm" | "rmi"), _)
                | (Some("prune"), _)
                | (Some("container" | "image" | "network"), Some("rm" | "prune")) => {
                    findings.push(Finding::new(
                        Severity::WritesState,
                        Category::Destructive,
                        format!("{head} rm"),
                    ));
                }
                (Some("compose"), Some(s @ ("down" | "stop" | "kill" | "restart")))
                | (Some(s @ ("stop" | "kill" | "down" | "restart")), _) => {
                    findings.push(Finding::new(
                        Severity::Elevated,
                        Category::Service,
                        format!("{head} {s}"),
                    ));
                }
                (Some("compose"), Some(s @ ("up" | "start"))) => {
                    findings.push(Finding::new(
                        Severity::WritesState,
                        Category::Service,
                        format!("{head} compose {s}"),
                    ));
                }
                _ => {}
            }
        }
        "kubectl" => match first_subcommand(args) {
            Some("delete") => {
                findings.push(Finding::new(
                    Severity::Critical,
                    Category::Destructive,
                    "kubectl delete",
                ));
            }
            Some("drain") => {
                findings.push(Finding::new(
                    Severity::Elevated,
                    Category::Service,
                    "kubectl drain",
                ));
            }
            Some(
                s @ ("apply" | "create" | "patch" | "replace" | "scale" | "cordon" | "uncordon"),
            ) => {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Service,
                    format!("kubectl {s}"),
                ));
            }
            _ => {}
        },
        "git" => match first_subcommand(args) {
            Some("reset") if args.iter().any(|a| a == "--hard") => {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Irreversible,
                    "git reset --hard",
                ));
            }
            Some("clean") if has_flag(args, &['f'], &["--force"]) => {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Irreversible,
                    "git clean -f",
                ));
            }
            // Exact `--force`/`-f` only: `--force-with-lease` is the safe variant
            // that refuses to clobber unseen remote work.
            Some("push") if args.iter().any(|a| a == "--force" || a == "-f") => {
                findings.push(Finding::new(
                    Severity::Elevated,
                    Category::Destructive,
                    "git push --force",
                ));
            }
            Some(s @ ("checkout" | "restore")) if has_flag(args, &['f'], &["--force"]) => {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Destructive,
                    format!("git {s} --force"),
                ));
            }
            Some("rm") => {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Destructive,
                    "git rm",
                ));
            }
            // status/log/diff/fetch/add/commit/pull/merge/rebase/stash/tag and a
            // non-forced checkout are normal low-risk workflow: no finding.
            _ => {}
        },
        "apt" | "apt-get" | "dnf" | "yum" | "zypper" => match first_subcommand(args) {
            Some(s @ ("remove" | "purge" | "autoremove" | "erase")) => {
                findings.push(Finding::new(
                    Severity::Elevated,
                    Category::Package,
                    format!("{head} {s}"),
                ));
            }
            Some(
                s @ ("install" | "upgrade" | "update" | "dist-upgrade" | "full-upgrade"
                | "reinstall"),
            ) => {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Package,
                    format!("{head} {s}"),
                ));
            }
            _ => {}
        },
        "pacman" => {
            // Operation is the leading char of the first short bundle. -Ss/-Si/
            // -Sl/-Sp/-Sg (search/info) and -Q* (query) are read-only; only -R
            // removes and a bare -S/-U installs.
            let bundle = args
                .iter()
                .find(|a| a.starts_with('-') && !a.starts_with("--"))
                .map(String::as_str)
                .unwrap_or("");
            let op = bundle.chars().nth(1);
            let mods: String = bundle.chars().skip(2).collect();
            let read = matches!(op, Some('Q' | 'F' | 'T'))
                || (op == Some('S')
                    && mods
                        .chars()
                        .any(|c| matches!(c, 's' | 'i' | 'l' | 'p' | 'g')));
            if read {
                // read-only
            } else if op == Some('R') {
                findings.push(Finding::new(
                    Severity::Elevated,
                    Category::Package,
                    "pacman -R",
                ));
            } else if matches!(op, Some('S' | 'U')) {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Package,
                    "pacman -S",
                ));
            }
        }
        "apk" => match first_subcommand(args) {
            Some("del") => findings.push(Finding::new(
                Severity::Elevated,
                Category::Package,
                "apk del",
            )),
            Some("add") | Some("upgrade") => {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Package,
                    "apk add",
                ));
            }
            _ => {}
        },
        "pip" | "pip3" | "npm" | "gem" | "cargo" | "snap" | "flatpak" | "brew" => {
            if positionals(args).iter().any(|p| {
                matches!(
                    p.as_str(),
                    "install" | "uninstall" | "remove" | "upgrade" | "update"
                )
            }) {
                findings.push(Finding::new(Severity::WritesState, Category::Package, head));
            }
        }
        "rsync" => {
            if args
                .iter()
                .any(|a| a == "--delete" || a.starts_with("--delete"))
            {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Destructive,
                    "rsync --delete",
                ));
            }
        }
        "crontab" => {
            if has_flag(args, &['r'], &[]) {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Destructive,
                    "crontab -r",
                ));
            }
        }
        "userdel" | "groupdel" => {
            findings.push(Finding::new(Severity::Elevated, Category::Privilege, head));
        }
        "useradd" | "usermod" | "groupadd" | "groupmod" | "passwd" | "chpasswd" => {
            findings.push(Finding::new(
                Severity::WritesState,
                Category::Privilege,
                head,
            ));
        }
        "iptables" | "ip6tables" | "nft" | "ufw" => {
            // Listing rules (-L, -nvL, `list`, `list ruleset`, `status`) is
            // read-only. Only mutating subforms flag.
            let flush = has_flag(args, &['F'], &["--flush"])
                || positionals(args)
                    .iter()
                    .any(|p| matches!(p.as_str(), "flush" | "reset"));
            let mutates = has_flag(
                args,
                &['A', 'I', 'D', 'R', 'X', 'N', 'P', 'Z'],
                &[
                    "--append",
                    "--insert",
                    "--delete",
                    "--replace",
                    "--new-chain",
                    "--policy",
                ],
            ) || positionals(args).iter().any(|p| {
                matches!(
                    p.as_str(),
                    "add"
                        | "insert"
                        | "delete"
                        | "replace"
                        | "create"
                        | "enable"
                        | "disable"
                        | "allow"
                        | "deny"
                        | "reject"
                        | "limit"
                )
            });
            if flush {
                findings.push(Finding::new(
                    Severity::Elevated,
                    Category::Service,
                    format!("{head} flush"),
                ));
            } else if mutates {
                findings.push(Finding::new(Severity::WritesState, Category::Service, head));
            }
        }
        "mount" | "umount" | "swapoff" | "swapon" => {
            findings.push(Finding::new(Severity::WritesState, Category::Service, head));
        }
        "tar" => {
            // Extraction overwrites files; create/list are read-ish (the archive
            // file write, if any, is covered by the redirect/output).
            if has_flag(args, &['x'], &["--extract", "--get"]) {
                findings.push(Finding::new(
                    Severity::WritesState,
                    Category::Destructive,
                    "tar -x",
                ));
            }
        }
        "sh" | "bash" | "dash" | "zsh" | "ksh" | "fish" | "python" | "python2" | "python3"
        | "perl" | "ruby" | "node" | "php" | "lua" => {
            // `-c <payload>` / perl-ruby `-e <payload>` runs an inline command
            // string: recurse one level so `bash -c "rm -rf /"` surfaces the rm.
            let payload = args
                .iter()
                .position(|a| a == "-c" || a == "-e")
                .and_then(|i| args.get(i + 1));
            if let Some(p) = payload {
                if depth == 0 {
                    findings.extend(classify(&segment(p), depth + 1));
                }
            } else if positionals(args).iter().any(|p| !p.is_empty()) {
                // Running a local script file: effect not verified.
                findings.push(Finding::new(Severity::WritesState, Category::Unknown, head));
            }
            // A bare interpreter reading stdin is handled by pipe detection.
        }
        "cat" | "head" | "tail" | "less" | "more" | "bat" | "xxd" | "hexdump" | "strings" => {
            if positionals(args).iter().any(|p| is_secret_path(p)) {
                findings.push(Finding::new(Severity::WritesState, Category::Secrets, head));
            }
        }
        _ => {
            if !READ_ONLY_HEADS.contains(&head) && !head.is_empty() {
                findings.push(Finding::new(Severity::WritesState, Category::Unknown, head));
            }
        }
    }
}

/// Analyse `command` for the Snippets IMPACT card. Pure and deterministic; runs
/// when the detail panel renders the selected snippet.
pub fn analyze_command(command: &str) -> CommandImpact {
    let segments = segment(command);
    let mut findings = classify(&segments, 0);
    dedup(&mut findings);
    CommandImpact { findings }
}

/// Remove duplicate findings (same severity + category + subject), preserving
/// first-seen order. Findings are few, so the O(n^2) scan is fine.
fn dedup(findings: &mut Vec<Finding>) {
    let mut seen: Vec<Finding> = Vec::new();
    findings.retain(|f| {
        if seen.contains(f) {
            false
        } else {
            seen.push(f.clone());
            true
        }
    });
}

#[cfg(test)]
#[path = "snippet_impact_tests.rs"]
mod tests;

#[cfg(test)]
mod _sudoers_trace_probe {
    use super::*;
    #[test]
    fn probe_sudoers_append() {
        let cmd = "echo 'attacker ALL=(ALL) NOPASSWD:ALL' >> /etc/sudoers";
        let segs = segment(cmd);
        eprintln!("SEGMENTS={:#?}", segs);
        let r = analyze_command(cmd);
        eprintln!("SUDOERS_VERDICT={:?}", r.verdict());
        eprintln!("SUDOERS_FINDINGS={:?}", r.findings);
    }
}

#[cfg(test)]
mod _find_exec_chmod_trace_probe {
    use super::*;
    #[test]
    fn probe_find_exec_chmod() {
        let cmds = [
            r#"find / -path '/proc' -prune -o -exec chmod 777 {} +"#,
            r#"find / -exec sh -c 'rm -rf "$1"' _ {} \;"#,
            r#"find . -exec mv {} /dev/null \;"#,
        ];
        for cmd in cmds {
            let segs = segment(cmd);
            eprintln!("CMD={:?}", cmd);
            for (i, s) in segs.iter().enumerate() {
                eprintln!(
                    "  SEG[{}] after_pipe={} words={:?} redirects_len={} subs={:?}",
                    i,
                    s.after_pipe,
                    s.words,
                    s.redirects.len(),
                    s.substitutions
                );
            }
            let r = analyze_command(cmd);
            eprintln!("  VERDICT={:?}", r.verdict());
            eprintln!("  FINDINGS={:?}", r.findings);
        }
    }
}
#[cfg(test)]
mod _case_probe {
    use super::*;
    #[test]
    fn probe_sudo_bash_c() {
        for cmd in ["sudo bash -c 'rm -rf /'", "sudo sh -c 'rm -rf /'"] {
            let segs = segment(cmd);
            let r = analyze_command(cmd);
            eprintln!("CMD={cmd:?}");
            for (i, s) in segs.iter().enumerate() {
                eprintln!(
                    "  SEG[{}] after_pipe={} words={:?} redirects_len={} subs={:?}",
                    i,
                    s.after_pipe,
                    s.words,
                    s.redirects.len(),
                    s.substitutions
                );
            }
            eprintln!("  VERDICT={:?}", r.verdict());
            eprintln!("  FINDINGS={:?}", r.findings);
        }
    }
}
