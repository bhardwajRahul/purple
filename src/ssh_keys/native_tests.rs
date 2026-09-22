use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine;

use super::*;

const PASSPHRASE: &str = "correct horse";

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn wire(value: &[u8]) -> Vec<u8> {
    let mut out = (value.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(value);
    out
}

fn blob(parts: &[&[u8]]) -> Vec<u8> {
    parts.iter().flat_map(|p| wire(p)).collect()
}

fn write_file(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

fn ed25519_blob() -> Vec<u8> {
    blob(&[b"ssh-ed25519", &[7u8; 32]])
}

fn p256_point() -> Vec<u8> {
    let mut point = vec![0x04];
    point.extend_from_slice(&[9u8; 64]);
    point
}

/// An RSA modulus of `bits` bits with its top bit set, as a shortest-form mpint.
fn rsa_modulus(bits: usize) -> Vec<u8> {
    let mut n = vec![0xffu8; bits / 8];
    n.insert(0, 0);
    n
}

// --- ssh-keygen parity: every case below is checked against the real tool ---

/// Whether `ssh-keygen` can be started. The parity tests need it.
fn keygen_available() -> bool {
    Command::new("ssh-keygen")
        .arg("-l")
        .arg("-f")
        .arg("/nonexistent")
        .output()
        .is_ok()
}

/// Generate a key pair with ssh-keygen. Returns the private key path.
fn generate(dir: &Path, name: &str, args: &[&str], passphrase: &str, comment: &str) -> PathBuf {
    let path = dir.join(name);
    let status = Command::new("ssh-keygen")
        .args(["-q", "-N", passphrase, "-C", comment, "-f"])
        .arg(&path)
        .args(args)
        .status()
        .expect("spawn ssh-keygen");
    assert!(status.success(), "ssh-keygen failed for {name} {args:?}");
    path
}

fn pub_of(private: &Path) -> PathBuf {
    let mut name = private.file_name().unwrap().to_os_string();
    name.push(".pub");
    private.with_file_name(name)
}

fn assert_public_parity(pub_path: &Path) {
    let native = inspect_public_key(pub_path)
        .unwrap_or_else(|| panic!("in-process reader skipped {}", pub_path.display()));
    let keygen = crate::ssh_keys::keygen_public_key_facts(pub_path)
        .unwrap_or_else(|| panic!("ssh-keygen could not read {}", pub_path.display()));
    assert_eq!(native, keygen, "facts differ for {}", pub_path.display());
}

fn assert_encryption_parity(private: &Path, expect_native: bool) {
    let keygen = crate::ssh_keys::keygen_private_key_encrypted(private);
    match private_key_encrypted(private) {
        Some(native) => assert_eq!(
            native,
            keygen,
            "encryption differs for {}",
            private.display()
        ),
        None => assert!(
            !expect_native,
            "in-process reader skipped {}",
            private.display()
        ),
    }
}

#[test]
fn parity_with_ssh_keygen_for_generated_keys() {
    if !keygen_available() {
        eprintln!("ssh-keygen not found, skipping parity test");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let cases: &[(&str, &[&str], &str, &str)] = &[
        ("ed", &["-t", "ed25519"], "", "eric@host"),
        ("ed_enc", &["-t", "ed25519"], PASSPHRASE, "eric@host"),
        ("ed_rounds", &["-t", "ed25519", "-a", "32"], PASSPHRASE, "x"),
        ("ed_no_comment", &["-t", "ed25519"], "", ""),
        (
            "ed_spaces",
            &["-t", "ed25519"],
            "",
            "eric@MacBook Pro (work)",
        ),
        ("rsa1024", &["-t", "rsa", "-b", "1024"], "", "old"),
        ("rsa2048", &["-t", "rsa", "-b", "2048"], "", "rsa"),
        (
            "rsa3072_enc",
            &["-t", "rsa", "-b", "3072"],
            PASSPHRASE,
            "rsa",
        ),
        ("rsa2056", &["-t", "rsa", "-b", "2056"], "", "odd size"),
        ("ec256", &["-t", "ecdsa", "-b", "256"], "", "ec"),
        ("ec384", &["-t", "ecdsa", "-b", "384"], PASSPHRASE, "ec"),
        ("ec521", &["-t", "ecdsa", "-b", "521"], "", "ec"),
    ];
    for (name, args, passphrase, comment) in cases {
        let private = generate(d, name, args, passphrase, comment);
        assert_public_parity(&pub_of(&private));
        assert_encryption_parity(&private, true);
    }
}

#[test]
fn parity_with_ssh_keygen_for_pem_private_keys() {
    if !keygen_available() {
        eprintln!("ssh-keygen not found, skipping parity test");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let cases: &[(&str, &[&str], &str)] = &[
        ("rsa_pem", &["-t", "rsa", "-b", "2048", "-m", "PEM"], ""),
        (
            "rsa_pem_enc",
            &["-t", "rsa", "-b", "2048", "-m", "PEM"],
            PASSPHRASE,
        ),
        ("ec_pem", &["-t", "ecdsa", "-m", "PEM"], ""),
        ("ec_pem_enc", &["-t", "ecdsa", "-m", "PEM"], PASSPHRASE),
    ];
    for (name, args, passphrase) in cases {
        let private = generate(d, name, args, passphrase, "pem");
        // The PEM flavor ssh-keygen writes depends on its crypto library.
        // An encrypted traditional RSA or EC key must be read in-process.
        let text = std::fs::read_to_string(&private).unwrap();
        let first = text.lines().next().unwrap_or("");
        let traditional = first == PEM_RSA_BEGIN || first == PEM_EC_BEGIN;
        assert_encryption_parity(&private, traditional && !passphrase.is_empty());
    }
}

#[test]
fn parity_with_ssh_keygen_for_densest_random_art_cell() {
    if !keygen_available() {
        eprintln!("ssh-keygen not found, skipping parity test");
        return;
    }
    // ssh-keygen draws '^' in the bottom-right cell of this key's art.
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(
        dir.path(),
        "dense.pub",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILT3HDj//X8tLVIswOTLvBxM4xUjTnMOcORCKUCtPAnZ dense14\n",
    );
    assert_public_parity(&path);
    let art = inspect_public_key(&path).unwrap().bishop_art;
    assert_eq!(art.lines().nth(9), Some("|            ..oO^|"));
}

#[test]
fn parity_with_ssh_keygen_random_art() {
    if !keygen_available() {
        eprintln!("ssh-keygen not found, skipping parity test");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for i in 0..24 {
        let private = generate(dir.path(), &format!("k{i}"), &["-t", "ed25519"], "", "art");
        assert_public_parity(&pub_of(&private));
    }
}

#[test]
fn parity_with_ssh_keygen_for_security_keys() {
    if !keygen_available() {
        eprintln!("ssh-keygen not found, skipping parity test");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();

    let ed = generate(d, "ed", &["-t", "ed25519"], "", "");
    let ed_line = std::fs::read_to_string(pub_of(&ed)).unwrap();
    let ed_blob = base64::engine::general_purpose::STANDARD
        .decode(ed_line.split(' ').nth(1).unwrap())
        .unwrap();
    let mut r = WireReader::new(&ed_blob);
    r.string().unwrap();
    let pk = r.string().unwrap();
    let sk_ed = blob(&[b"sk-ssh-ed25519@openssh.com", pk, b"ssh:"]);
    let path = write_file(
        d,
        "sk_ed.pub",
        &format!("sk-ssh-ed25519@openssh.com {} yubikey\n", b64(&sk_ed)),
    );
    assert_public_parity(&path);

    let ec = generate(d, "ec", &["-t", "ecdsa", "-b", "256"], "", "");
    let ec_line = std::fs::read_to_string(pub_of(&ec)).unwrap();
    let ec_blob = base64::engine::general_purpose::STANDARD
        .decode(ec_line.split(' ').nth(1).unwrap())
        .unwrap();
    let mut r = WireReader::new(&ec_blob);
    r.string().unwrap();
    r.string().unwrap();
    let q = r.string().unwrap();
    let sk_ec = blob(&[
        b"sk-ecdsa-sha2-nistp256@openssh.com",
        b"nistp256",
        q,
        b"ssh:",
    ]);
    let path = write_file(
        d,
        "sk_ec.pub",
        &format!("sk-ecdsa-sha2-nistp256@openssh.com {} token\n", b64(&sk_ec)),
    );
    assert_public_parity(&path);
}

#[test]
fn discover_keys_reads_plain_keys_in_process_and_certificates_via_ssh_keygen() {
    if !keygen_available() {
        eprintln!("ssh-keygen not found, skipping parity test");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let user = generate(d, "id_ed25519", &["-t", "ed25519"], "", "user");
    let ca_dir = tempfile::tempdir().unwrap();
    let ca = generate(ca_dir.path(), "ca", &["-t", "ed25519"], "", "ca");
    let status = Command::new("ssh-keygen")
        .args(["-q", "-s"])
        .arg(&ca)
        .args(["-I", "test-id", "-n", "user"])
        .arg(pub_of(&user))
        .status()
        .unwrap();
    assert!(status.success());
    let cert = d.join("id_ed25519-cert.pub");
    assert!(inspect_public_key(&cert).is_none());

    let keys = crate::ssh_keys::discover_keys(None, d, &[]);
    let names: Vec<&str> = keys.iter().map(|k| k.name.as_str()).collect();
    assert_eq!(names, vec!["id_ed25519", "id_ed25519-cert"]);
    let plain = &keys[0];
    assert_eq!(plain.key_type, "ED25519");
    assert!(!plain.is_certificate);
    assert_eq!(plain.bishop_lines().len(), 11);
    let certificate = &keys[1];
    assert_eq!(certificate.key_type, "ED25519-CERT");
    assert!(certificate.is_certificate);
}

#[test]
fn discover_keys_checks_other_private_key_formats_via_ssh_keygen() {
    if !keygen_available() {
        eprintln!("ssh-keygen not found, skipping parity test");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let pkcs8 = &["-t", "rsa", "-b", "2048", "-m", "PKCS8"];
    let plain = generate(d, "a_plain", pkcs8, "", "plain");
    let locked = generate(d, "b_locked", pkcs8, PASSPHRASE, "locked");
    assert_eq!(private_key_encrypted(&plain), None);
    assert_eq!(private_key_encrypted(&locked), None);

    let keys = crate::ssh_keys::discover_keys(None, d, &[]);
    let flags: Vec<(&str, bool)> = keys
        .iter()
        .map(|k| (k.name.as_str(), k.encrypted))
        .collect();
    assert_eq!(flags, vec![("a_plain", false), ("b_locked", true)]);
}

// --- public key line handling ---

#[test]
fn public_key_comment_defaults_like_ssh_keygen() {
    let line = format!("ssh-ed25519 {}", b64(&ed25519_blob()));
    let facts = parse_public_key_line(&line).unwrap();
    assert_eq!(facts.comment, "no comment");
    assert_eq!(facts.key_type, "ED25519");
    assert_eq!(facts.bits, "256");
    assert!(facts.fingerprint.starts_with("SHA256:"));

    let hashed = format!("ssh-ed25519 {} #note", b64(&ed25519_blob()));
    assert_eq!(
        parse_public_key_line(&hashed).unwrap().comment,
        "no comment"
    );

    let spaced = format!("ssh-ed25519\t{}  \t eric@host  ", b64(&ed25519_blob()));
    assert_eq!(parse_public_key_line(&spaced).unwrap().comment, "eric@host");
}

#[test]
fn public_key_comment_with_escaped_bytes_is_left_to_ssh_keygen() {
    let line = format!("ssh-ed25519 {} caf\u{e9}", b64(&ed25519_blob()));
    assert!(parse_public_key_line(&line).is_none());
    let tab = format!("ssh-ed25519 {} a\tb", b64(&ed25519_blob()));
    assert!(parse_public_key_line(&tab).is_none());
}

#[test]
fn first_key_line_skips_blank_and_comment_lines() {
    let text = "\n# a note\n   \r\n  ssh-ed25519 AAAA x\r\nssh-rsa BBBB\n";
    assert_eq!(first_key_line(text), Some("ssh-ed25519 AAAA x"));
}

#[test]
fn first_key_line_leaves_private_keys_to_ssh_keygen() {
    assert_eq!(
        first_key_line("-----BEGIN OPENSSH PRIVATE KEY-----\n"),
        None
    );
    assert_eq!(first_key_line("\n\n"), None);
}

#[test]
fn public_key_type_must_match_blob_type() {
    let line = format!("ssh-rsa {} x", b64(&ed25519_blob()));
    assert!(parse_public_key_line(&line).is_none());
}

#[test]
fn public_key_with_options_prefix_is_left_to_ssh_keygen() {
    let line = format!("no-pty ssh-ed25519 {} x", b64(&ed25519_blob()));
    assert!(parse_public_key_line(&line).is_none());
}

#[test]
fn public_key_with_bad_base64_is_left_to_ssh_keygen() {
    assert!(parse_public_key_line("ssh-ed25519 !!!! x").is_none());
    assert!(parse_public_key_line("ssh-ed25519").is_none());
    assert!(parse_public_key_line("ssh-ed25519 ").is_none());
}

#[test]
fn inspect_public_key_rejects_oversized_file() {
    let dir = tempfile::tempdir().unwrap();
    let line = format!("ssh-ed25519 {} x\n", b64(&ed25519_blob()));
    let padding = "#".repeat(MAX_KEY_FILE_BYTES as usize);
    let path = write_file(dir.path(), "big.pub", &format!("{padding}\n{line}"));
    assert!(inspect_public_key(&path).is_none());
    let path = write_file(dir.path(), "small.pub", &line);
    assert!(inspect_public_key(&path).is_some());
}

#[test]
fn inspect_public_key_missing_file_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert!(inspect_public_key(&dir.path().join("absent.pub")).is_none());
}

// --- blob parsing ---

#[test]
fn blob_ed25519_requires_32_byte_key() {
    assert!(parse_blob(&ed25519_blob()).is_some());
    assert!(parse_blob(&blob(&[b"ssh-ed25519", &[7u8; 31]])).is_none());
}

#[test]
fn blob_with_trailing_bytes_is_rejected() {
    let mut b = ed25519_blob();
    b.push(0);
    assert!(parse_blob(&b).is_none());
}

#[test]
fn blob_truncated_is_rejected() {
    let b = ed25519_blob();
    assert!(parse_blob(&b[..b.len() - 1]).is_none());
    assert!(parse_blob(&[0, 0]).is_none());
}

#[test]
fn blob_rsa_reports_modulus_bits() {
    let b = blob(&[b"ssh-rsa", &[1, 0, 1], &rsa_modulus(2048)]);
    let parsed = parse_blob(&b).unwrap();
    assert_eq!(parsed.short_name, "RSA");
    assert_eq!(parsed.bits, 2048);
}

#[test]
fn blob_rsa_below_openssh_minimum_is_left_to_ssh_keygen() {
    let b = blob(&[b"ssh-rsa", &[1, 0, 1], &rsa_modulus(768)]);
    assert!(parse_blob(&b).is_none());
}

#[test]
fn blob_rsa_non_shortest_mpint_is_rejected() {
    let mut n = rsa_modulus(2048);
    n.insert(0, 0);
    let b = blob(&[b"ssh-rsa", &[1, 0, 1], &n]);
    assert!(parse_blob(&b).is_none());
    let padded_e = blob(&[b"ssh-rsa", &[0, 1, 0, 1], &rsa_modulus(2048)]);
    assert!(parse_blob(&padded_e).is_none());
}

#[test]
fn blob_rsa_negative_mpint_is_rejected() {
    let b = blob(&[b"ssh-rsa", &[1, 0, 1], &[0xff; 256]]);
    assert!(parse_blob(&b).is_none());
}

#[test]
fn blob_rsa_oversized_mpint_is_rejected() {
    let mut n = vec![0x7f; MAX_BIGNUM_BYTES + 2];
    n[0] = 0x01;
    let b = blob(&[b"ssh-rsa", &[1, 0, 1], &n]);
    assert!(parse_blob(&b).is_none());

    // One byte over the cap is only allowed as a leading zero.
    let mut n = vec![0xab; MAX_BIGNUM_BYTES + 1];
    n[0] = 0x01;
    let b = blob(&[b"ssh-rsa", &[1, 0, 1], &n]);
    assert!(parse_blob(&b).is_none());
}

#[test]
fn blob_rsa_largest_openssh_modulus_is_accepted() {
    let b = blob(&[b"ssh-rsa", &[1, 0, 1], &rsa_modulus(16384)]);
    assert_eq!(parse_blob(&b).unwrap().bits, 16384);
}

#[test]
fn blob_ecdsa_checks_curve_and_point() {
    let good = blob(&[b"ecdsa-sha2-nistp256", b"nistp256", &p256_point()]);
    let parsed = parse_blob(&good).unwrap();
    assert_eq!(parsed.short_name, "ECDSA");
    assert_eq!(parsed.bits, 256);

    let wrong_curve = blob(&[b"ecdsa-sha2-nistp256", b"nistp384", &p256_point()]);
    assert!(parse_blob(&wrong_curve).is_none());

    let mut compressed = p256_point();
    compressed[0] = 0x02;
    let compressed = blob(&[b"ecdsa-sha2-nistp256", b"nistp256", &compressed]);
    assert!(parse_blob(&compressed).is_none());

    let short = blob(&[b"ecdsa-sha2-nistp256", b"nistp256", &p256_point()[..64]]);
    assert!(parse_blob(&short).is_none());
}

#[test]
fn blob_ecdsa_point_sizes_per_curve() {
    assert_eq!(EcCurve::P256.point_len(), 65);
    assert_eq!(EcCurve::P384.point_len(), 97);
    assert_eq!(EcCurve::P521.point_len(), 133);
}

#[test]
fn blob_security_key_application_rejects_any_nul() {
    let good = blob(&[b"sk-ssh-ed25519@openssh.com", &[7u8; 32], b"ssh:"]);
    assert_eq!(parse_blob(&good).unwrap().short_name, "ED25519-SK");
    let inner = blob(&[b"sk-ssh-ed25519@openssh.com", &[7u8; 32], b"ss\0h:"]);
    assert!(parse_blob(&inner).is_none());
    // OpenSSH drops a trailing NUL and hashes the re-encoded key, so the raw
    // blob would give a different fingerprint.
    let trailing = blob(&[b"sk-ssh-ed25519@openssh.com", &[7u8; 32], b"ssh:\0"]);
    assert!(parse_blob(&trailing).is_none());
    let ec = blob(&[
        b"sk-ecdsa-sha2-nistp256@openssh.com",
        b"nistp256",
        &p256_point(),
        b"ssh:\0",
    ]);
    assert!(parse_blob(&ec).is_none());
}

#[test]
fn blob_certificates_and_dsa_are_left_to_ssh_keygen() {
    let cert = blob(&[b"ssh-ed25519-cert-v01@openssh.com", &[1u8; 32], &[7u8; 32]]);
    assert!(parse_blob(&cert).is_none());
    let dsa = blob(&[b"ssh-dss", &[1], &[1], &[1], &[1]]);
    assert!(parse_blob(&dsa).is_none());
}

#[test]
fn bit_length_counts_significant_bits() {
    assert_eq!(bit_length(&[]), 0);
    assert_eq!(bit_length(&[0, 0]), 0);
    assert_eq!(bit_length(&[1]), 1);
    assert_eq!(bit_length(&[0x80]), 8);
    assert_eq!(bit_length(&[0x01, 0x00]), 9);
}

// --- random art ---

#[test]
fn art_title_matches_openssh_fallbacks() {
    assert_eq!(art_title("ED25519", "256"), "[ED25519 256]");
    assert_eq!(art_title("ED25519-SK", "256"), "[ED25519-SK 256]");
    assert_eq!(art_title("ED25519-CERT", "256"), "[ED25519-CERT]");
    // Longer than the field: cut the way a 17-byte C buffer cuts it.
    assert_eq!(art_title("ED25519-SK-CERT", "256"), "[ED25519-SK-CERT");
}

#[test]
fn art_border_centers_label() {
    assert_eq!(art_border("[ED25519 256]"), "+--[ED25519 256]--+");
    assert_eq!(art_border("[SHA256]"), "+----[SHA256]-----+");
    assert_eq!(art_border("[RSA 4096]"), "+---[RSA 4096]----+");
}

#[test]
fn random_art_marks_dense_cells_like_openssh() {
    // Zero bytes walk the bishop into the top-left corner and keep it
    // there, so that cell hits the highest density mark ('^'). The final
    // 0xff byte steps it back out so the corner is not overwritten by E.
    let mut digest = vec![0u8; 10];
    digest.push(0xff);
    let art = random_art(&digest, "ED25519", "256");
    let lines: Vec<&str> = art.lines().collect();
    assert_eq!(lines.len(), 11);
    assert_eq!(&lines[1][..2], "|^");
}

// --- private key headers ---

/// Line width of the base64 body in the generated key files.
const ARMOR_LINE_WIDTH: usize = 70;

/// Private section with matching check integers plus one block of padding.
const PLAIN_PRIVATE_SECTION: [u8; 16] = [1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 5, 6, 7, 8];

fn armor(body: &[u8]) -> String {
    let encoded = b64(body);
    let wrapped: Vec<&str> = encoded
        .as_bytes()
        .chunks(ARMOR_LINE_WIDTH)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    format!(
        "{OPENSSH_KEY_BEGIN}\n{}\n{OPENSSH_KEY_END}\n",
        wrapped.join("\n")
    )
}

/// The decoded body of an OpenSSH private key file (PROTOCOL.key).
fn openssh_body(cipher: &[u8], kdf: &[u8], nkeys: u32, public: &[u8], private: &[u8]) -> Vec<u8> {
    let mut body = OPENSSH_KEY_MAGIC.to_vec();
    body.extend(wire(cipher));
    body.extend(wire(kdf));
    body.extend(wire(b""));
    body.extend(nkeys.to_be_bytes());
    body.extend(wire(public));
    body.extend(wire(private));
    body
}

fn openssh_private_key(cipher: &[u8], kdf: &[u8], private: &[u8]) -> String {
    armor(&openssh_body(cipher, kdf, 1, &ed25519_blob(), private))
}

fn encrypted_of(dir: &Path, name: &str, text: &str) -> Option<bool> {
    private_key_encrypted(&write_file(dir, name, text))
}

#[test]
fn openssh_private_key_cipher_decides_encryption() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let plain = openssh_private_key(b"none", b"none", &PLAIN_PRIVATE_SECTION);
    assert_eq!(encrypted_of(d, "plain", &plain), Some(false));
    let enc = openssh_private_key(b"aes256-ctr", b"bcrypt", &[9u8; 32]);
    assert_eq!(encrypted_of(d, "enc", &enc), Some(true));
}

#[test]
fn openssh_private_key_bad_framing_is_left_to_ssh_keygen() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let section = &PLAIN_PRIVATE_SECTION;
    let mut mismatched = PLAIN_PRIVATE_SECTION;
    mismatched[0] = 0xff;
    let mut trailing = openssh_body(b"none", b"none", 1, &ed25519_blob(), section);
    trailing.push(0);
    let cert = blob(&[b"ssh-ed25519-cert-v01@openssh.com", &[1u8; 32], &[7u8; 32]]);
    let plain = openssh_private_key(b"none", b"none", section);
    let cases: Vec<(&str, String)> = vec![
        ("kdf", openssh_private_key(b"none", b"bcrypt", section)),
        (
            "checkint",
            openssh_private_key(b"none", b"none", &mismatched),
        ),
        (
            "short",
            openssh_private_key(b"none", b"none", &section[..4]),
        ),
        (
            "unaligned",
            openssh_private_key(b"none", b"none", &section[..12]),
        ),
        ("trailing", armor(&trailing)),
        (
            "nkeys",
            armor(&openssh_body(b"none", b"none", 2, &ed25519_blob(), section)),
        ),
        (
            "cert",
            armor(&openssh_body(b"none", b"none", 1, &cert, section)),
        ),
        ("nul", openssh_private_key(b"none\0", b"none", section)),
        ("crlf", plain.replace('\n', "\r\n")),
        ("no_final_newline", plain.trim_end().to_string()),
        ("leading_blank", format!("\n{plain}")),
        (
            "garbage",
            format!("{OPENSSH_KEY_BEGIN}\n!!!!\n{OPENSSH_KEY_END}\n"),
        ),
        ("magic", armor(b"nope")),
    ];
    for (name, text) in cases {
        assert_eq!(encrypted_of(d, name, &text), None, "{name}");
    }
}

#[test]
fn openssh_private_key_garbage_body_is_left_to_ssh_keygen() {
    let dir = tempfile::tempdir().unwrap();
    let garbage = write_file(
        dir.path(),
        "garbage",
        &format!("{OPENSSH_KEY_BEGIN}\n!!!!\n{OPENSSH_KEY_END}\n"),
    );
    assert_eq!(private_key_encrypted(&garbage), None);
    let wrong_magic = write_file(
        dir.path(),
        "magic",
        &format!("{OPENSSH_KEY_BEGIN}\n{}\n{OPENSSH_KEY_END}\n", b64(b"nope")),
    );
    assert_eq!(private_key_encrypted(&wrong_magic), None);
}

/// Base64 of a DER SEQUENCE holding one INTEGER 0: framed like a key, but
/// not one `ssh-keygen` can load.
const NOT_A_KEY_DER_B64: &str = "MAMCAQA=";

fn pem(kind: &str, headers: &str, body: &str) -> String {
    format!(
        "-----BEGIN {kind} PRIVATE KEY-----\n{headers}{body}\n-----END {kind} PRIVATE KEY-----\n"
    )
}

#[test]
fn pem_private_key_with_proc_type_header_is_encrypted() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let headers = "Proc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,00\n\n";
    assert_eq!(
        encrypted_of(d, "ec_enc", &pem("EC", headers, "MIIE")),
        Some(true)
    );
    assert_eq!(
        encrypted_of(d, "rsa_enc", &pem("RSA", headers, "MIIE")),
        Some(true)
    );
}

#[test]
fn pem_private_key_without_proc_type_header_is_left_to_ssh_keygen() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let cases = [
        ("rsa", pem("RSA", "", NOT_A_KEY_DER_B64)),
        ("ec", pem("EC", "", NOT_A_KEY_DER_B64)),
        (
            "spaced_proc_type",
            pem("RSA", "Proc-Type:  4,ENCRYPTED\n\n", "MIIE"),
        ),
        (
            "late_proc_type",
            pem(
                "RSA",
                "DEK-Info: AES-128-CBC,00\nProc-Type: 4,ENCRYPTED\n\n",
                "MIIE",
            ),
        ),
        (
            "header_only",
            "-----BEGIN RSA PRIVATE KEY-----\n".to_string(),
        ),
    ];
    for (name, text) in cases {
        assert_eq!(encrypted_of(d, name, &text), None, "{name}");
    }
}

#[cfg(unix)]
#[test]
fn unloadable_pem_private_key_matches_ssh_keygen() {
    use std::os::unix::fs::PermissionsExt;
    if !keygen_available() {
        eprintln!("ssh-keygen not found, skipping parity test");
        return;
    }
    // ssh-keygen refuses to load this, so it reports the key as encrypted.
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(dir.path(), "tiny", &pem("RSA", "", NOT_A_KEY_DER_B64));
    let owner_only = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(&path, owner_only).unwrap();
    assert!(crate::ssh_keys::keygen_private_key_encrypted(&path));
    assert!(crate::ssh_keys::private_key_encrypted(&path));
}

#[cfg(unix)]
fn make_fifo(path: &Path) {
    let status = Command::new("mkfifo").arg(path).status().unwrap();
    assert!(status.success());
}

#[cfg(unix)]
#[test]
fn fifo_is_skipped_without_blocking() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("id_fifo");
    make_fifo(&fifo);
    assert_eq!(private_key_encrypted(&fifo), None);
    assert!(inspect_public_key(&fifo).is_none());
    assert!(!crate::ssh_keys::private_key_encrypted(&fifo));
}

#[test]
fn private_key_that_is_a_directory_is_not_encrypted() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("id_dir");
    std::fs::create_dir(&sub).unwrap();
    assert!(!crate::ssh_keys::private_key_encrypted(&sub));
}

#[test]
fn other_private_key_formats_are_left_to_ssh_keygen() {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in [
        (
            "pkcs8",
            "-----BEGIN PRIVATE KEY-----\nMIIE\n-----END PRIVATE KEY-----\n",
        ),
        (
            "pkcs8_enc",
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIE\n-----END ENCRYPTED PRIVATE KEY-----\n",
        ),
        (
            "dsa",
            "-----BEGIN DSA PRIVATE KEY-----\nMIIE\n-----END DSA PRIVATE KEY-----\n",
        ),
        ("putty", "PuTTY-User-Key-File-3: ssh-ed25519\n"),
        ("empty", ""),
    ] {
        let path = write_file(dir.path(), name, text);
        assert_eq!(private_key_encrypted(&path), None, "{name}");
    }
    assert_eq!(private_key_encrypted(&dir.path().join("absent")), None);
}
