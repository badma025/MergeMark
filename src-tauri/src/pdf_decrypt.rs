//! Reading a paper its producer encrypted for permissions only: an owner
//! password and an empty user password, as AQA's papers are. PDFium opens
//! such a file itself, but the font catalog reads the font dictionaries
//! (`/Differences`, `/Widths`) through lopdf, which decrypts RC4 only and
//! drops the object streams of an AES-encrypted file, the fonts with them:
//! every control-range code of AQA Jun 2021's maths fonts then went
//! unnamed, its minus signs among them. A file encrypted with AES-128 (PDF
//! 32000-1 §7.6: V 4, R 4, crypt filter AESV2) is loaded through a filter
//! that decrypts each object as it is read. Every encrypted paper in the
//! corpus is of that kind; any other (RC4, AES-256, another crypt filter) is
//! read as lopdf reads it. A stream lopdf fills after the filter has run
//! (its /Length an object inside an object stream) stays encrypted: no
//! corpus paper has one.

use md5::{Digest, Md5};
use pdf_extract::{Document, Object, ObjectId};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

/// The document at `path`, its strings and streams decrypted when the file
/// is encrypted with AES-128 and its user password is empty; as lopdf reads
/// it otherwise. `None` when it cannot be read at all.
pub fn load(path: &Path) -> Option<Document> {
    let doc = Document::load(path).ok()?;
    let Some(key) = file_key(&doc) else { return Some(doc) };
    // Objects inside an object stream are decrypted with their stream, not
    // one by one; the encryption dictionary and cross-reference streams
    // are not encrypted at all.
    let packed: HashSet<u32> = doc
        .reference_table
        .entries
        .iter()
        .filter_map(|(&n, e)| matches!(e, pdf_extract::xref::XrefEntry::Compressed { .. }).then_some(n))
        .collect();
    let encrypt = doc.trailer.get(b"Encrypt").and_then(Object::as_reference).ok();
    // (lopdf's filter is a plain function: it reads the key from here, one
    // encrypted load at a time, and the key goes when the load ends, even
    // by a panic.)
    let _one = LOADING.lock().unwrap_or_else(|p| p.into_inner());
    // (Metadata stays plain when /EncryptMetadata is false.)
    let plain_metadata = doc.get_encrypted().is_ok_and(|e| e.get(b"EncryptMetadata").and_then(Object::as_bool).ok() == Some(false));
    *FILTER.lock().unwrap_or_else(|p| p.into_inner()) = Some(Filter { key, packed, encrypt, plain_metadata });
    let _clear = ClearFilter;
    Document::load_filtered(path, decrypt_object).ok().or(Some(doc))
}

struct Filter {
    key: [u8; 16],
    packed: HashSet<u32>,
    encrypt: Option<ObjectId>,
    plain_metadata: bool,
}

static LOADING: Mutex<()> = Mutex::new(());
static FILTER: Mutex<Option<Filter>> = Mutex::new(None);

/// Clears the key when an encrypted load ends.
struct ClearFilter;

impl Drop for ClearFilter {
    fn drop(&mut self) {
        *FILTER.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

/// lopdf's load filter: an object read at the top level is decrypted in
/// place (the reader keeps it; the returned copy is dropped), an object
/// read from an object stream is returned as it is (the reader keeps the
/// copy).
fn decrypt_object(id: ObjectId, obj: &mut Object) -> Option<(ObjectId, Object)> {
    let filter = FILTER.lock().unwrap_or_else(|p| p.into_inner());
    let Some(f) = filter.as_ref() else { return Some((id, obj.clone())) };
    if f.packed.contains(&id.0) {
        return Some((id, std::mem::replace(obj, Object::Null)));
    }
    let plain = obj.as_stream().is_ok_and(|s| s.dict.type_is(b"XRef") || f.plain_metadata && s.dict.type_is(b"Metadata"));
    if Some(id) != f.encrypt && !plain {
        decrypt_in(obj, &object_key(&f.key, id));
    }
    Some((id, Object::Null))
}

/// Strings and stream content decrypted, through dictionaries and arrays.
fn decrypt_in(obj: &mut Object, key: &[u8; 16]) {
    match obj {
        Object::String(bytes, _) => {
            if let Some(plain) = aes_cbc_decrypt(key, bytes) {
                *bytes = plain;
            }
        }
        Object::Array(items) => items.iter_mut().for_each(|o| decrypt_in(o, key)),
        Object::Dictionary(d) => d.iter_mut().for_each(|(_, o)| decrypt_in(o, key)),
        Object::Stream(s) => {
            s.dict.iter_mut().for_each(|(_, o)| decrypt_in(o, key));
            if let Some(plain) = aes_cbc_decrypt(key, &s.content) {
                s.set_content(plain);
            }
        }
        _ => {}
    }
}

/// The padding a password is completed with (Algorithm 2).
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C,
    0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// The file key of a standard-security file encrypted with AES-128 (V 4,
/// R 4, AESV2 as the crypt filter of its streams and strings), for the
/// empty user password (Algorithm 2), when it is that password's: the key
/// reproduces /U (Algorithms 5 and 6). `None` for any other encryption, or
/// none.
fn file_key(doc: &Document) -> Option<[u8; 16]> {
    let enc = doc.get_encrypted().ok()?;
    let int = |k: &[u8]| enc.get(k).and_then(Object::as_i64).ok();
    let aes = enc
        .get(b"CF")
        .and_then(Object::as_dict)
        .and_then(|cf| cf.get(b"StdCF"))
        .and_then(Object::as_dict)
        .and_then(|f| f.get(b"CFM"))
        .and_then(Object::as_name)
        .is_ok_and(|m| m == b"AESV2");
    // (A missing /StmF or /StrF is the Identity filter: that data is plain.)
    let std_cf = |k: &[u8]| enc.get(k).and_then(Object::as_name).ok() == Some(b"StdCF".as_slice());
    if enc.get(b"Filter").and_then(Object::as_name).ok() != Some(b"Standard".as_slice()) || int(b"V") != Some(4) || int(b"R") != Some(4) || !aes || !std_cf(b"StmF") || !std_cf(b"StrF") {
        return None;
    }
    let owner = enc.get(b"O").and_then(Object::as_str).ok()?;
    let user = enc.get(b"U").and_then(Object::as_str).ok()?;
    let permissions = int(b"P")? as i32;
    let id = doc.trailer.get(b"ID").and_then(Object::as_array).ok()?.first()?.as_str().ok()?;
    let mut h = Md5::new();
    h.update(PAD);
    h.update(owner);
    h.update(permissions.to_le_bytes());
    h.update(id);
    if enc.get(b"EncryptMetadata").and_then(Object::as_bool).ok() == Some(false) {
        h.update([0xFF; 4]);
    }
    let mut key: [u8; 16] = h.finalize().into();
    for _ in 0..50 {
        key = Md5::digest(key).into();
    }
    let mut check: Vec<u8> = Md5::new().chain_update(PAD).chain_update(id).finalize().to_vec();
    for i in 0..20u8 {
        let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
        check = rc4(&k, &check);
    }
    (user.len() >= 16 && check[..] == user[..16]).then_some(key)
}

/// The key of one object (Algorithm 1, with AES's salt).
fn object_key(file_key: &[u8; 16], id: ObjectId) -> [u8; 16] {
    let mut h = Md5::new();
    h.update(file_key);
    h.update(&id.0.to_le_bytes()[..3]);
    h.update(id.1.to_le_bytes());
    h.update(b"sAlT");
    h.finalize().into()
}

/// RC4, for the check of the user password (Algorithm 5).
fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|&b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

/// AES-128-CBC: the first block is the IV; the plaintext is padded to the
/// block size (PKCS #5). Padding that is no such padding is kept, as PDFium
/// keeps it. `None` when the data is no ciphertext: shorter than the IV, or
/// not in whole blocks.
fn aes_cbc_decrypt(key: &[u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 16 || data.len() % 16 != 0 {
        return None;
    }
    let mut plain = cbc_decrypt(key, data[..16].try_into().ok()?, &data[16..]);
    let pad = plain.last().map_or(0, |&b| b as usize);
    if (1..=16).contains(&pad) && plain[plain.len() - pad..].iter().all(|&b| b as usize == pad) {
        plain.truncate(plain.len() - pad);
    }
    Some(plain)
}

fn cbc_decrypt(key: &[u8; 16], iv: [u8; 16], data: &[u8]) -> Vec<u8> {
    let round_keys = expand_key(key);
    let mut prev = iv;
    let mut out = Vec::with_capacity(data.len());
    for block in data.chunks_exact(16) {
        let cipher: [u8; 16] = block.try_into().unwrap_or([0; 16]);
        let mut b = cipher;
        decrypt_block(&mut b, &round_keys);
        for (x, p) in b.iter_mut().zip(prev) {
            *x ^= p;
        }
        out.extend_from_slice(&b);
        prev = cipher;
    }
    out
}

// ── AES-128 (FIPS 197), decryption only ─────────────────────────────────────

/// Multiplication in GF(2⁸) modulo x⁸ + x⁴ + x³ + x + 1.
fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        let high = a & 0x80;
        a <<= 1;
        if high != 0 {
            a ^= 0x1B;
        }
        b >>= 1;
    }
    p
}

/// The S-box and its inverse: the multiplicative inverse in GF(2⁸) under
/// the affine map, computed once.
static SBOXES: std::sync::LazyLock<([u8; 256], [u8; 256])> = std::sync::LazyLock::new(|| {
    let mut sbox = [0u8; 256];
    let mut inverse = [0u8; 256];
    for x in 0..=255u8 {
        let y = if x == 0 { 0 } else { (1..=255u8).find(|&c| gmul(x, c) == 1).unwrap_or(0) };
        let s = y ^ y.rotate_left(1) ^ y.rotate_left(2) ^ y.rotate_left(3) ^ y.rotate_left(4) ^ 0x63;
        sbox[x as usize] = s;
        inverse[s as usize] = x;
    }
    (sbox, inverse)
});

/// InvMixColumns' products: each byte times 14, 11, 13 and 9.
static MIX: std::sync::LazyLock<[[u8; 256]; 4]> = std::sync::LazyLock::new(|| std::array::from_fn(|k| std::array::from_fn(|x| gmul(x as u8, [14, 11, 13, 9][k]))));

fn expand_key(key: &[u8; 16]) -> [[u8; 16]; 11] {
    let sbox = &SBOXES.0;
    let mut w = [[0u8; 4]; 44];
    for (i, word) in w.iter_mut().take(4).enumerate() {
        word.copy_from_slice(&key[4 * i..4 * i + 4]);
    }
    let mut rcon = 1u8;
    for i in 4..44 {
        let mut t = w[i - 1];
        if i % 4 == 0 {
            t.rotate_left(1);
            for b in t.iter_mut() {
                *b = sbox[*b as usize];
            }
            t[0] ^= rcon;
            rcon = gmul(rcon, 2);
        }
        for j in 0..4 {
            w[i][j] = w[i - 4][j] ^ t[j];
        }
    }
    let mut round_keys = [[0u8; 16]; 11];
    for (r, rk) in round_keys.iter_mut().enumerate() {
        for c in 0..4 {
            rk[4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
        }
    }
    round_keys
}

/// One block through the inverse cipher; the state is column-major (byte
/// r + 4c is row r of column c).
fn decrypt_block(s: &mut [u8; 16], round_keys: &[[u8; 16]; 11]) {
    let inverse = &SBOXES.1;
    let [m14, m11, m13, m9] = &*MIX;
    let add = |s: &mut [u8; 16], k: &[u8; 16]| s.iter_mut().zip(k).for_each(|(x, k)| *x ^= k);
    add(s, &round_keys[10]);
    for round in (0..10).rev() {
        let t = *s;
        for r in 1..4 {
            for c in 0..4 {
                s[r + 4 * c] = t[r + 4 * ((c + 4 - r) % 4)];
            }
        }
        for b in s.iter_mut() {
            *b = inverse[*b as usize];
        }
        add(s, &round_keys[round]);
        if round > 0 {
            for c in 0..4 {
                let a = [s[4 * c] as usize, s[4 * c + 1] as usize, s[4 * c + 2] as usize, s[4 * c + 3] as usize];
                s[4 * c] = m14[a[0]] ^ m11[a[1]] ^ m13[a[2]] ^ m9[a[3]];
                s[4 * c + 1] = m9[a[0]] ^ m14[a[1]] ^ m11[a[2]] ^ m13[a[3]];
                s[4 * c + 2] = m13[a[0]] ^ m9[a[1]] ^ m14[a[2]] ^ m11[a[3]];
                s[4 * c + 3] = m11[a[0]] ^ m13[a[1]] ^ m9[a[2]] ^ m14[a[3]];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// The inverse cipher undoes FIPS 197's AES-128 example (Appendix C.1),
    /// and CBC decryption NIST SP 800-38A's (F.2.2, its first two blocks).
    #[test]
    fn aes_128_decrypts_the_standard_examples() {
        let key: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let mut block: [u8; 16] = hex("69c4e0d86a7b0430d8cdb78070b4c55a").try_into().unwrap();
        decrypt_block(&mut block, &expand_key(&key));
        assert_eq!(block.to_vec(), hex("00112233445566778899aabbccddeeff"));
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        let iv: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
        let plain = cbc_decrypt(&key, iv, &hex("7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b2"));
        assert_eq!(plain, hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51"));
    }

    /// RC4 gives the published test vector ("Key", "Plaintext").
    #[test]
    fn rc4_encrypts_the_published_example() {
        assert_eq!(rc4(b"Key", b"Plaintext"), hex("bbf316e8d940af0ad3"));
    }

    /// Data that is no ciphertext is left alone, and padding that is no
    /// padding is kept: SP 800-38A's two blocks end in 0x51.
    #[test]
    fn malformed_ciphertext_is_never_trusted() {
        let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
        assert_eq!(aes_cbc_decrypt(&key, &[1u8; 15]), None);
        assert_eq!(aes_cbc_decrypt(&key, &[1u8; 40]), None);
        assert_eq!(aes_cbc_decrypt(&key, &[1u8; 16]), Some(Vec::new()));
        let data = hex("000102030405060708090a0b0c0d0e0f7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b2");
        assert_eq!(aes_cbc_decrypt(&key, &data), Some(hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51")));
    }

    /// The key is computed only for AES-128 with the empty user password:
    /// AQA Jun 2021's /U proves its key, and a /U it does not reproduce, no
    /// file identifier, another revision or another crypt filter for its
    /// strings gives none.
    #[test]
    fn the_file_key_is_the_empty_passwords_only() {
        let Some(path) = crate::test_support::corpus_paper("papers/used/Maths_A-level_Papers_AQA__June_2021_QP.pdf") else { return };
        let doc = Document::load(&path).expect("the paper is read");
        assert!(file_key(&doc).is_some());
        let enc_id = doc.trailer.get(b"Encrypt").and_then(Object::as_reference).expect("an encryption dictionary");
        let altered = |f: &dyn Fn(&mut Document)| {
            let mut d = doc.clone();
            f(&mut d);
            file_key(&d)
        };
        assert_eq!(
            altered(&|d| {
                let enc = d.get_object_mut(enc_id).and_then(Object::as_dict_mut).unwrap();
                let mut u = enc.get(b"U").and_then(Object::as_str).unwrap().to_vec();
                u[0] ^= 1;
                enc.set("U", Object::String(u, pdf_extract::StringFormat::Literal));
            }),
            None
        );
        assert_eq!(altered(&|d| d.trailer.remove(b"ID").map(drop).unwrap_or(())), None);
        assert_eq!(altered(&|d| d.get_object_mut(enc_id).and_then(Object::as_dict_mut).unwrap().set("R", 3)), None);
        assert_eq!(altered(&|d| d.get_object_mut(enc_id).and_then(Object::as_dict_mut).unwrap().set("StrF", Object::Name(b"Identity".to_vec()))), None);
    }

    /// An AES-encrypted paper's font dictionaries are read: AQA Jun 2021
    /// names its maths font's code 2 "C0" (its minus) in `/Differences`.
    #[test]
    fn an_aes_encrypted_papers_fonts_are_read() {
        let Some(path) = crate::test_support::corpus_paper("papers/used/Maths_A-level_Papers_AQA__June_2021_QP.pdf") else { return };
        let doc = load(&path).expect("the paper is read");
        assert_eq!(doc.get_pages().len(), 36);
        let differences = doc.objects.values().filter_map(|o| o.as_dict().ok()).filter(|d| d.type_is(b"Encoding")).filter_map(|d| d.get(b"Differences").ok()).count();
        assert!(differences > 0, "the fonts' encodings are read");
    }
}
