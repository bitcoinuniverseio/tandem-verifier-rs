//! JCS canonical agreement tuple signing with Ed25519.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::{Deserialize, Serialize};
use tandem_core::{HeightRoots, display_hash};
use zeroize::Zeroizing;

use crate::config::ReleaseIdentity;

/// Exact cross-indexer agreement tuple.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgreementTuple {
    /// Schema identifier.
    pub schema: String,
    /// Bound protocol identifier.
    pub protocol_id: String,
    /// Canonical height as an unsigned decimal string.
    pub height: String,
    /// Canonical block hash in display order.
    pub block_hash: String,
    /// Event root.
    pub event_root: String,
    /// Object-state root.
    pub object_state_root: String,
    /// Chained root.
    pub chained_root: String,
    /// Founding count as an unsigned decimal string.
    pub founding_created: String,
    /// Object count as an unsigned decimal string.
    pub all_objects: String,
    /// Active count as an unsigned decimal string.
    pub active_objects: String,
    /// Parser source commit.
    pub parser_commit: String,
    /// Indexer source commit.
    pub indexer_commit: String,
    /// Parser release artifact digest.
    pub parser_binary_sha256: String,
    /// Indexer release artifact digest.
    pub indexer_binary_sha256: String,
}

impl AgreementTuple {
    /// Build the exact tuple from a persisted height and release identity.
    pub fn from_roots(protocol_id: &str, roots: &HeightRoots, release: &ReleaseIdentity) -> Self {
        Self {
            schema: "urn:tandem:agreement-tuple".to_owned(),
            protocol_id: protocol_id.to_owned(),
            height: roots.height.to_string(),
            block_hash: display_hash(roots.block_hash),
            event_root: roots.event_root.to_hex(),
            object_state_root: roots.object_state_root.to_hex(),
            chained_root: roots.chained_root.to_hex(),
            founding_created: roots.counters.founding_created.to_string(),
            all_objects: roots.counters.all_objects.to_string(),
            active_objects: roots.counters.active_objects.to_string(),
            parser_commit: release.parser_commit.clone(),
            indexer_commit: release.indexer_commit.clone(),
            parser_binary_sha256: release.parser_binary_sha256.to_hex(),
            indexer_binary_sha256: release.indexer_binary_sha256.to_hex(),
        }
    }

    /// Serialize using RFC 8785 JSON Canonicalization Scheme.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        serde_jcs::to_vec(self).context("cannot canonicalize agreement tuple")
    }
}

/// Signed agreement envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgreementEnvelope {
    /// Schema identifier.
    pub schema: String,
    /// Public signer key identifier.
    pub key_id: String,
    /// Signed tuple.
    pub tuple: AgreementTuple,
    /// Ed25519 signature as 128 lowercase hexadecimal characters.
    pub signature: String,
}

/// In-memory signing key loaded from a restricted seed file.
pub struct AgreementSigner {
    key_id: String,
    signing_key: SigningKey,
}

impl AgreementSigner {
    /// Load an exact 32-byte seed encoded as 64 hexadecimal characters.
    pub fn load(path: &Path, key_id: String) -> Result<Self> {
        let text = Zeroizing::new(
            std::fs::read_to_string(path)
                .with_context(|| format!("cannot read signing key file {}", path.display()))?,
        );
        let trimmed = text.trim();
        ensure!(
            trimmed.len() == 64,
            "signing key file must contain exactly 64 hexadecimal characters"
        );
        let mut seed = Zeroizing::new([0_u8; 32]);
        hex::decode_to_slice(trimmed, seed.as_mut()).context("signing seed is not hexadecimal")?;
        Ok(Self {
            key_id,
            signing_key: SigningKey::from_bytes(&seed),
        })
    }

    /// Return the public verifying key as lowercase hexadecimal.
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.signing_key.verifying_key().to_bytes())
    }

    /// Sign one exact canonical tuple.
    pub fn sign(&self, tuple: AgreementTuple) -> Result<AgreementEnvelope> {
        let signature = self.signing_key.sign(&tuple.canonical_bytes()?);
        Ok(AgreementEnvelope {
            schema: "urn:tandem:agreement-envelope".to_owned(),
            key_id: self.key_id.clone(),
            tuple,
            signature: hex::encode(signature.to_bytes()),
        })
    }

    /// Verify an envelope against an explicit public key.
    pub fn verify(envelope: &AgreementEnvelope, public_key: &VerifyingKey) -> Result<()> {
        let bytes = envelope.tuple.canonical_bytes()?;
        let signature_bytes: [u8; 64] = hex::decode(&envelope.signature)
            .context("signature is not hexadecimal")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("signature must be 64 bytes"))?;
        let signature = Signature::from_bytes(&signature_bytes);
        public_key
            .verify(&bytes, &signature)
            .context("agreement signature failed verification")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tandem_core::{Counters, Hash32, HeightRoots};

    #[test]
    fn jcs_signatures_are_stable_and_verifiable() {
        let signer = AgreementSigner {
            key_id: "test-key".to_owned(),
            signing_key: SigningKey::from_bytes(&[7; 32]),
        };
        let tuple = AgreementTuple {
            schema: "urn:tandem:agreement-tuple".to_owned(),
            protocol_id:
                "tndm:regtest:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_owned(),
            height: "1".to_owned(),
            block_hash: "00".repeat(32),
            event_root: "01".repeat(32),
            object_state_root: "02".repeat(32),
            chained_root: "03".repeat(32),
            founding_created: "0".to_owned(),
            all_objects: "0".to_owned(),
            active_objects: "0".to_owned(),
            parser_commit: "a".repeat(40),
            indexer_commit: "b".repeat(40),
            parser_binary_sha256: "c".repeat(64),
            indexer_binary_sha256: "d".repeat(64),
        };
        let first = signer.sign(tuple.clone()).expect("sign");
        let second = signer.sign(tuple).expect("sign");
        assert_eq!(first.signature, second.signature);
        AgreementSigner::verify(&first, &signer.signing_key.verifying_key()).expect("verify");

        let _unused_roots = HeightRoots {
            height: 0,
            block_hash: Hash32::ZERO,
            event_root: Hash32::ZERO,
            object_state_root: Hash32::ZERO,
            chained_root: Hash32::ZERO,
            counters: Counters::default(),
        };
    }
}
