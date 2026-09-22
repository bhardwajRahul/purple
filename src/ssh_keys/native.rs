//! In-process reading of SSH key files. Covers the key formats most people
//! have, so the key list does not start two `ssh-keygen` processes per key.
//! Anything this module does not recognize returns `None`, and the caller
//! then asks `ssh-keygen` instead.

use std::io::Read;
use std::path::Path;

use base64::Engine;
use sha2::{Digest, Sha256};

use super::PublicKeyFacts;

/// Largest key file read in-process. Real key files are a few KiB; anything
/// bigger is left to `ssh-keygen`.
const MAX_KEY_FILE_BYTES: u64 = 64 * 1024;

/// OpenSSH refuses RSA keys below this modulus size (`SSH_RSA_MINIMUM_MODULUS_SIZE`).
const RSA_MIN_BITS: u32 = 1024;

/// OpenSSH caps a bignum at 16384 bits, plus one leading zero byte.
const MAX_BIGNUM_BYTES: usize = 16384 / 8;

const ED25519_PK_BYTES: usize = 32;

/// `ssh-keygen` reports every Ed25519 key, security keys included, as 256 bits.
const ED25519_BITS: u32 = 256;

/// First byte of an uncompressed EC point, the only form OpenSSH accepts.
const EC_POINT_UNCOMPRESSED: u8 = 0x04;

/// Block size OpenSSH checks the private section against when the cipher is `none`.
const OPENSSH_NONE_BLOCK_SIZE: usize = 8;

/// Width and height of the canonical OpenSSH random-art field.
const ART_COLS: usize = 17;
const ART_ROWS: usize = 9;

const OPENSSH_KEY_MAGIC: &[u8] = b"openssh-key-v1\0";
const OPENSSH_KEY_BEGIN: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";
const OPENSSH_KEY_END: &str = "-----END OPENSSH PRIVATE KEY-----";
const PEM_RSA_BEGIN: &str = "-----BEGIN RSA PRIVATE KEY-----";
const PEM_EC_BEGIN: &str = "-----BEGIN EC PRIVATE KEY-----";
const PEM_ENCRYPTED_HEADER: &str = "Proc-Type: 4,ENCRYPTED";

/// Comment `ssh-keygen -l` prints for a key without one.
const NO_COMMENT: &str = "no comment";

/// Read a `.pub` file and return what `ssh-keygen -lv -E sha256` would
/// report for its first key.
pub(super) fn inspect_public_key(path: &Path) -> Option<PublicKeyFacts> {
    let text = read_capped_utf8(path)?;
    let line = first_key_line(&text)?;
    parse_public_key_line(line)
}

/// Whether `ssh-keygen -y -P ""` would fail on this private key, read from
/// the file header. `None` for formats this module does not recognize.
pub(super) fn private_key_encrypted(path: &Path) -> Option<bool> {
    let text = read_capped_utf8(path)?;
    let first = text.lines().next()?.trim_end();
    match first {
        OPENSSH_KEY_BEGIN => openssh_key_encrypted(&text),
        PEM_RSA_BEGIN | PEM_EC_BEGIN => pem_key_encrypted(&text),
        _ => None,
    }
}

/// Read a regular file as UTF-8. Anything else (a FIFO, a device, a
/// directory) is skipped, since opening a FIFO blocks.
fn read_capped_utf8(path: &Path) -> Option<String> {
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_KEY_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_KEY_FILE_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The first line `ssh-keygen -l` would treat as a key: blank lines and
/// `#` lines are skipped. A file that opens with a private key is left to
/// `ssh-keygen`.
fn first_key_line(text: &str) -> Option<&str> {
    for (idx, raw) in text.split('\n').enumerate() {
        let line = raw.split(['\r', '\n']).next().unwrap_or("");
        let line = line.trim_start_matches([' ', '\t']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if idx == 0 && line.contains("PRIVATE KEY") {
            return None;
        }
        return Some(line);
    }
    None
}

fn parse_public_key_line(line: &str) -> Option<PublicKeyFacts> {
    let is_ws = |c: char| c == ' ' || c == '\t';
    let (type_name, rest) = line.split_once(is_ws)?;
    let rest = rest.trim_start_matches(is_ws);
    let (blob_b64, rest) = rest.split_once(is_ws).unwrap_or((rest, ""));
    if blob_b64.is_empty() {
        return None;
    }
    let blob = base64::engine::general_purpose::STANDARD
        .decode(blob_b64)
        .ok()?;
    let parsed = parse_blob(&blob)?;
    if parsed.name != type_name {
        return None;
    }
    let comment = comment_from(rest)?;

    let digest = Sha256::digest(&blob);
    let fingerprint = format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    );
    let bits = parsed.bits.to_string();
    let bishop_art = random_art(&digest, parsed.short_name, &bits);
    Some(PublicKeyFacts {
        bits,
        fingerprint,
        comment,
        key_type: parsed.short_name.to_string(),
        bishop_art,
    })
}

/// Comment as `ssh-keygen -l` prints it. Text that `ssh-keygen` would
/// escape on output is left to `ssh-keygen`.
fn comment_from(rest: &str) -> Option<String> {
    let rest = rest.trim_matches([' ', '\t']);
    if rest.is_empty() || rest.starts_with('#') {
        return Some(NO_COMMENT.to_string());
    }
    if !rest.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return None;
    }
    Some(rest.to_string())
}

struct ParsedBlob {
    name: &'static str,
    short_name: &'static str,
    bits: u32,
}

/// Parse a plain public key blob with the framing, size and bignum checks
/// OpenSSH applies when it loads one. EC points are not checked against the
/// curve. Certificates and DSA keys return `None`.
fn parse_blob(blob: &[u8]) -> Option<ParsedBlob> {
    let mut r = WireReader::new(blob);
    let name = r.string()?;
    let parsed = match name {
        b"ssh-rsa" => {
            let _e = r.mpint()?;
            let n = r.mpint()?;
            let bits = bit_length(n);
            if bits < RSA_MIN_BITS {
                return None;
            }
            ParsedBlob {
                name: "ssh-rsa",
                short_name: "RSA",
                bits,
            }
        }
        b"ssh-ed25519" => {
            r.ed25519_key()?;
            ParsedBlob {
                name: "ssh-ed25519",
                short_name: "ED25519",
                bits: ED25519_BITS,
            }
        }
        b"sk-ssh-ed25519@openssh.com" => {
            r.ed25519_key()?;
            r.cstring()?;
            ParsedBlob {
                name: "sk-ssh-ed25519@openssh.com",
                short_name: "ED25519-SK",
                bits: ED25519_BITS,
            }
        }
        b"ecdsa-sha2-nistp256" => ecdsa(&mut r, "ecdsa-sha2-nistp256", EcCurve::P256)?,
        b"ecdsa-sha2-nistp384" => ecdsa(&mut r, "ecdsa-sha2-nistp384", EcCurve::P384)?,
        b"ecdsa-sha2-nistp521" => ecdsa(&mut r, "ecdsa-sha2-nistp521", EcCurve::P521)?,
        b"sk-ecdsa-sha2-nistp256@openssh.com" => {
            r.ec_point(EcCurve::P256)?;
            r.cstring()?;
            ParsedBlob {
                name: "sk-ecdsa-sha2-nistp256@openssh.com",
                short_name: "ECDSA-SK",
                bits: EcCurve::P256.bits(),
            }
        }
        _ => return None,
    };
    if !r.is_empty() {
        return None;
    }
    Some(parsed)
}

#[derive(Clone, Copy)]
enum EcCurve {
    P256,
    P384,
    P521,
}

impl EcCurve {
    fn name(self) -> &'static [u8] {
        match self {
            EcCurve::P256 => b"nistp256",
            EcCurve::P384 => b"nistp384",
            EcCurve::P521 => b"nistp521",
        }
    }

    fn bits(self) -> u32 {
        match self {
            EcCurve::P256 => 256,
            EcCurve::P384 => 384,
            EcCurve::P521 => 521,
        }
    }

    /// Size of an uncompressed point: a 0x04 marker plus two coordinates.
    fn point_len(self) -> usize {
        1 + 2 * (self.bits() as usize).div_ceil(8)
    }
}

fn ecdsa(r: &mut WireReader<'_>, name: &'static str, curve: EcCurve) -> Option<ParsedBlob> {
    r.ec_point(curve)?;
    Some(ParsedBlob {
        name,
        short_name: "ECDSA",
        bits: curve.bits(),
    })
}

/// Number of significant bits in a big-endian unsigned integer.
fn bit_length(n: &[u8]) -> u32 {
    let Some(pos) = n.iter().position(|&b| b != 0) else {
        return 0;
    };
    let significant = &n[pos..];
    (significant.len() as u32 - 1) * 8 + (8 - significant[0].leading_zeros())
}

/// Reader for the SSH wire encoding (RFC 4251 section 5).
struct WireReader<'a> {
    buf: &'a [u8],
}

impl<'a> WireReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    fn string(&mut self) -> Option<&'a [u8]> {
        let (len, rest) = self.buf.split_first_chunk::<4>()?;
        let len = u32::from_be_bytes(*len) as usize;
        if rest.len() < len {
            return None;
        }
        let (value, rest) = rest.split_at(len);
        self.buf = rest;
        Some(value)
    }

    fn u32(&mut self) -> Option<u32> {
        let (value, rest) = self.buf.split_first_chunk::<4>()?;
        self.buf = rest;
        Some(u32::from_be_bytes(*value))
    }

    /// A string OpenSSH reads as text. OpenSSH drops a trailing NUL and
    /// re-encodes the key, which changes the fingerprint, so any NUL is refused.
    fn cstring(&mut self) -> Option<&'a [u8]> {
        let value = self.string()?;
        (!value.contains(&0)).then_some(value)
    }

    /// A positive mpint in its shortest form. OpenSSH re-encodes the key
    /// before hashing it, so only the shortest form hashes to the same
    /// fingerprint as the raw blob.
    fn mpint(&mut self) -> Option<&'a [u8]> {
        let value = self.string()?;
        if value.first().is_some_and(|&b| b & 0x80 != 0) {
            return None;
        }
        if value.len() > MAX_BIGNUM_BYTES + 1
            || (value.len() == MAX_BIGNUM_BYTES + 1 && value[0] != 0)
        {
            return None;
        }
        let stripped = &value[value.iter().take_while(|&&b| b == 0).count()..];
        let needs_pad = stripped.first().is_some_and(|&b| b & 0x80 != 0);
        if value.len() != stripped.len() + usize::from(needs_pad) {
            return None;
        }
        Some(stripped)
    }

    fn ed25519_key(&mut self) -> Option<()> {
        (self.string()?.len() == ED25519_PK_BYTES).then_some(())
    }

    fn ec_point(&mut self, curve: EcCurve) -> Option<()> {
        if self.cstring()? != curve.name() {
            return None;
        }
        let point = self.string()?;
        (point.len() == curve.point_len() && point[0] == EC_POINT_UNCOMPRESSED).then_some(())
    }
}

/// The random-art block `ssh-keygen -lv` prints, as 11 lines joined with
/// `\n`. Follows `fingerprint_randomart` in OpenSSH `sshkey.c`.
fn random_art(digest: &[u8], short_name: &str, bits: &str) -> String {
    let grid = super::drunken_bishop_grid(digest, ART_COLS, ART_ROWS);
    let mut lines = Vec::with_capacity(ART_ROWS + 2);
    lines.push(art_border(&art_title(short_name, bits)));
    for row in &grid {
        let body: String = row.iter().map(|&c| super::bishop_char(c)).collect();
        lines.push(format!("|{body}|"));
    }
    lines.push(art_border("[SHA256]"));
    lines.join("\n")
}

/// `[TYPE BITS]`, or `[TYPE]` when that does not fit, cut to the field
/// width the way OpenSSH's fixed-size buffer cuts it.
fn art_title(short_name: &str, bits: &str) -> String {
    let full = format!("[{short_name} {bits}]");
    let mut title = if full.len() > ART_COLS {
        format!("[{short_name}]")
    } else {
        full
    };
    title.truncate(ART_COLS - 1);
    title
}

fn art_border(label: &str) -> String {
    let left = (ART_COLS - label.len()) / 2;
    let right = ART_COLS - left - label.len();
    format!("+{}{label}{}+", "-".repeat(left), "-".repeat(right))
}

/// Read an OpenSSH-format private key (PROTOCOL.key) the way
/// `private2_uudecode` and `private2_decrypt` in OpenSSH `sshkey.c` do.
/// A cipher means encrypted. An unencrypted key must be framed correctly,
/// otherwise `ssh-keygen` decides.
fn openssh_key_encrypted(text: &str) -> Option<bool> {
    let body = text.strip_prefix(OPENSSH_KEY_BEGIN)?.strip_prefix('\n')?;
    let end = body.find(&format!("\n{OPENSSH_KEY_END}\n"))?;
    let encoded: String = body[..end].chars().filter(|&c| c != '\n').collect();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let rest = decoded.strip_prefix(OPENSSH_KEY_MAGIC)?;
    let mut r = WireReader::new(rest);
    if r.cstring()? != b"none" {
        return Some(true);
    }
    if r.cstring()? != b"none" {
        return None;
    }
    r.string()?;
    if r.u32()? != 1 {
        return None;
    }
    parse_blob(r.string()?)?;
    let private_len = r.u32()? as usize;
    let private = r.buf;
    if private_len < OPENSSH_NONE_BLOCK_SIZE
        || !private_len.is_multiple_of(OPENSSH_NONE_BLOCK_SIZE)
        || private.len() != private_len
    {
        return None;
    }
    let mut p = WireReader::new(private);
    (p.u32()? == p.u32()?).then_some(false)
}

/// Read a traditional PEM RSA or EC private key. The canonical `Proc-Type`
/// header right after the BEGIN line means encrypted. Whether an
/// unencrypted PEM key loads depends on its contents (key size, curve,
/// structure), so `ssh-keygen` decides those.
fn pem_key_encrypted(text: &str) -> Option<bool> {
    let second = text.lines().nth(1)?.trim_end();
    (second == PEM_ENCRYPTED_HEADER).then_some(true)
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
