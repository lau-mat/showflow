//! Account-based caller access that works without internet during the show.
//!
//! The cloud signs a **pass**: "this user, on the device holding this key, may call this show until
//! then". A host that once learned the cloud's public key checks passes offline. A pass alone is
//! not enough: the device must also sign the host's per-connection nonce with its own key (the
//! **proof**), so a copied pass or a replayed claim is useless.
//!
//! Wire formats (all base64url without padding):
//! pass  = `<json body>.<cloud signature over the body>`
//! proof = `<device signature over session id and nonce>`

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ed25519_dalek::{Signature, Signer};
pub use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

pub const PASS_VERSION: u32 = 1;
/// How long a pass is good for. Apps renew theirs well before this while they are online.
pub const PASS_LIFETIME_SECONDS: i64 = 14 * 24 * 60 * 60;
/// Allowed difference between the cloud's clock and the host's.
const CLOCK_SKEW_SECONDS: i64 = 5 * 60;
/// Far longer than any real pass; refused before it is decoded.
const MAX_PASS_CHARS: usize = 2048;

// What is signed starts with its purpose, so a signature can never be used as the other kind.
const PASS_DOMAIN: &[u8] = b"showflow-caller-pass-v1\n";
const PROOF_DOMAIN: &[u8] = b"showflow-caller-proof-v1\n";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PassBody {
    pub v: u32,
    pub show_id: String,
    pub user_id: String,
    pub username: String,
    /// The public key of the device the pass was issued to.
    pub device_key: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PassError {
    /// Not a pass, or not signed by the cloud this host trusts.
    Invalid,
    OtherShow,
    Expired,
    /// The device could not show that the pass is its own.
    BadProof,
}

impl std::fmt::Display for PassError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "That caller pass is not valid",
            Self::OtherShow => "That caller pass is for another show",
            Self::Expired => "Your caller pass has expired. Go online to renew it.",
            Self::BadProof => "This device could not prove the caller pass is its own",
        })
    }
}

impl std::error::Error for PassError {}

/// A key pair from 32 secret random bytes.
pub fn signing_key(seed: [u8; 32]) -> SigningKey {
    SigningKey::from_bytes(&seed)
}

/// The shareable half of a key, as it is stored and sent.
pub fn public_key_text(key: &SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())
}

pub fn parse_public_key(text: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = URL_SAFE_NO_PAD.decode(text).ok()?.try_into().ok()?;
    let key = VerifyingKey::from_bytes(&bytes).ok()?;
    // Low-order keys would let one signature pass for any message.
    (!key.is_weak()).then_some(key)
}

fn message(domain: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut message = domain.to_vec();
    message.extend_from_slice(&parts.join(&b'\n'));
    message
}

fn signature(text: &str) -> Option<Signature> {
    Signature::from_slice(&URL_SAFE_NO_PAD.decode(text).ok()?).ok()
}

pub fn sign_pass(cloud: &SigningKey, body: &PassBody) -> String {
    // A struct of strings and integers always serializes.
    let json = serde_json::to_vec(body).expect("a pass body serializes");
    let signed = cloud.sign(&message(PASS_DOMAIN, &[&json]));
    format!("{}.{}", URL_SAFE_NO_PAD.encode(&json), URL_SAFE_NO_PAD.encode(signed.to_bytes()))
}

/// Checks the cloud's signature, the show and the lifetime. The body is only read after the
/// signature holds.
pub fn verify_pass(cloud: &VerifyingKey, pass: &str, show_id: &str, now: i64) -> Result<PassBody, PassError> {
    if pass.len() > MAX_PASS_CHARS {
        return Err(PassError::Invalid);
    }
    let (body, signed) = pass.split_once('.').ok_or(PassError::Invalid)?;
    let json = URL_SAFE_NO_PAD.decode(body).map_err(|_| PassError::Invalid)?;
    let signed = signature(signed).ok_or(PassError::Invalid)?;
    cloud.verify_strict(&message(PASS_DOMAIN, &[&json]), &signed).map_err(|_| PassError::Invalid)?;
    let body: PassBody = serde_json::from_slice(&json).map_err(|_| PassError::Invalid)?;
    if body.v != PASS_VERSION {
        return Err(PassError::Invalid);
    }
    if body.show_id != show_id {
        return Err(PassError::OtherShow);
    }
    if now >= body.expires_at || now + CLOCK_SKEW_SECONDS < body.issued_at {
        return Err(PassError::Expired);
    }
    Ok(body)
}

/// The device's answer to one connection: it signs that session's id and nonce.
pub fn sign_proof(device: &SigningKey, session_id: &str, nonce: &str) -> String {
    let signed = device.sign(&message(PROOF_DOMAIN, &[session_id.as_bytes(), nonce.as_bytes()]));
    URL_SAFE_NO_PAD.encode(signed.to_bytes())
}

pub fn verify_proof(body: &PassBody, proof: &str, session_id: &str, nonce: &str) -> Result<(), PassError> {
    let key = parse_public_key(&body.device_key).ok_or(PassError::BadProof)?;
    let signed = signature(proof).ok_or(PassError::BadProof)?;
    key.verify_strict(&message(PROOF_DOMAIN, &[session_id.as_bytes(), nonce.as_bytes()]), &signed)
        .map_err(|_| PassError::BadProof)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn body(device: &SigningKey) -> PassBody {
        PassBody {
            v: PASS_VERSION,
            show_id: "show-1".into(),
            user_id: "user-1".into(),
            username: "Sam".into(),
            device_key: public_key_text(device),
            issued_at: NOW,
            expires_at: NOW + PASS_LIFETIME_SECONDS,
        }
    }

    #[test]
    fn a_signed_pass_verifies_for_its_show_while_it_lasts() {
        let (cloud, device) = (signing_key([1; 32]), signing_key([2; 32]));
        let trusted = parse_public_key(&public_key_text(&cloud)).unwrap();
        let pass = sign_pass(&cloud, &body(&device));
        assert_eq!(verify_pass(&trusted, &pass, "show-1", NOW + 60), Ok(body(&device)));
        assert_eq!(verify_pass(&trusted, &pass, "show-2", NOW), Err(PassError::OtherShow));
        assert_eq!(verify_pass(&trusted, &pass, "show-1", NOW + PASS_LIFETIME_SECONDS), Err(PassError::Expired));
        // A host clock slightly behind the cloud's is tolerated, a day behind is not.
        assert!(verify_pass(&trusted, &pass, "show-1", NOW - 60).is_ok());
        assert_eq!(verify_pass(&trusted, &pass, "show-1", NOW - 86_400), Err(PassError::Expired));
    }

    #[test]
    fn forged_edited_and_malformed_passes_are_refused() {
        let (cloud, other, device) = (signing_key([1; 32]), signing_key([3; 32]), signing_key([2; 32]));
        let trusted = cloud.verifying_key();
        let refused = |pass: &str| assert_eq!(verify_pass(&trusted, pass, "show-1", NOW), Err(PassError::Invalid), "{pass}");
        refused(&sign_pass(&other, &body(&device)));
        let genuine = sign_pass(&cloud, &body(&device));
        let (_, signature) = genuine.split_once('.').unwrap();
        let mut edited = body(&device);
        edited.username = "Mallory".into();
        refused(&format!("{}.{signature}", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&edited).unwrap())));
        let mut future_version = body(&device);
        future_version.v = 2;
        refused(&sign_pass(&cloud, &future_version));
        for junk in ["", ".", "abc", "abc.def", "!!.!!"] {
            refused(junk);
        }
        refused(&"a".repeat(MAX_PASS_CHARS + 1));
    }

    #[test]
    fn the_proof_ties_a_claim_to_the_device_and_to_one_connection() {
        let (device, thief) = (signing_key([2; 32]), signing_key([4; 32]));
        let body = body(&device);
        let proof = sign_proof(&device, "session-1", "nonce-1");
        assert_eq!(verify_proof(&body, &proof, "session-1", "nonce-1"), Ok(()));
        for (session, nonce) in [("session-1", "nonce-2"), ("session-2", "nonce-1")] {
            assert_eq!(verify_proof(&body, &proof, session, nonce), Err(PassError::BadProof), "replayed");
        }
        assert_eq!(verify_proof(&body, &sign_proof(&thief, "session-1", "nonce-1"), "session-1", "nonce-1"), Err(PassError::BadProof));
        assert_eq!(verify_proof(&body, "junk", "session-1", "nonce-1"), Err(PassError::BadProof));
    }

    #[test]
    fn public_keys_round_trip_and_weak_or_malformed_ones_are_refused() {
        let key = signing_key([9; 32]);
        assert_eq!(parse_public_key(&public_key_text(&key)), Some(key.verifying_key()));
        let mut identity = [0u8; 32];
        identity[0] = 1;
        for bad in ["", "abc", &URL_SAFE_NO_PAD.encode(identity), &URL_SAFE_NO_PAD.encode([0u8; 31])] {
            assert!(parse_public_key(bad).is_none(), "{bad}");
        }
    }
}
