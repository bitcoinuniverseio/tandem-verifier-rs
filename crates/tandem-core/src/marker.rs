//! Broad marker candidate detection and strict payload parsing.

use serde::{Deserialize, Serialize};

use crate::{Hash32, MAGIC, Opcode, Reason};

/// A broadly detected Tandem marker candidate.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MarkerCandidate {
    /// Actual output index.
    pub vout: u32,
    /// Declared data length.
    pub declared_len: usize,
    /// Physically present pushed bytes, truncated to the declared length.
    #[serde(with = "hex_bytes")]
    pub payload: Vec<u8>,
    /// Complete push prefix was present.
    pub prefix_complete: bool,
    /// Declared bytes were physically present.
    pub payload_complete: bool,
    /// No script bytes followed the declared push.
    pub no_trailing_bytes: bool,
    /// Push used its shortest encoding.
    pub minimal_push: bool,
}

impl MarkerCandidate {
    /// Return physically readable opcode byte.
    pub fn opcode_byte(&self) -> Option<u8> {
        self.payload.get(6).copied()
    }

    /// Return true for a foreign INIT candidate.
    pub fn is_init_candidate(&self) -> bool {
        self.opcode_byte() == Some(0)
    }

    /// Validate structural encoding and parse the readable payload header.
    ///
    /// # Errors
    ///
    /// Returns the stable marker-encoding reason when the push is incomplete,
    /// nonminimal, oversized, terminated incorrectly, or has the wrong exact
    /// length for a defined opcode.
    pub fn parse(&self) -> Result<ParsedMarker, MarkerError> {
        if !self.prefix_complete
            || !(7..=80).contains(&self.declared_len)
            || !self.payload_complete
            || !self.no_trailing_bytes
            || !self.minimal_push
        {
            return Err(MarkerError::new(Reason::BadMarkerEncodingOrLength));
        }
        let version = self.payload[4];
        let network = self.payload[5];
        let opcode_byte = self.payload[6];
        let opcode = Opcode::from_byte(opcode_byte);
        if opcode.is_some_and(|value| value.payload_len() != self.payload.len()) {
            return Err(MarkerError::new(Reason::BadMarkerEncodingOrLength));
        }
        Ok(ParsedMarker {
            vout: self.vout,
            version,
            network,
            opcode_byte,
            opcode,
            payload: self.payload.clone(),
        })
    }
}

/// A structurally valid Tandem marker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParsedMarker {
    /// Marker output index.
    pub vout: u32,
    /// Payload version byte.
    pub version: u8,
    /// Payload network byte.
    pub network: u8,
    /// Raw opcode byte.
    pub opcode_byte: u8,
    /// Defined opcode when known.
    pub opcode: Option<Opcode>,
    /// Exact pushed payload.
    #[serde(with = "hex_bytes")]
    pub payload: Vec<u8>,
}

impl ParsedMarker {
    /// Read a little-endian `u32` field.
    ///
    /// # Panics
    ///
    /// Panics when a caller requests bytes outside a previously validated
    /// operation payload. Reducer callers use only fixed fields for that opcode.
    pub fn u32_at(&self, start: usize) -> u32 {
        u32::from_le_bytes(
            self.payload[start..start + 4]
                .try_into()
                .expect("known marker field has exact width"),
        )
    }

    /// Read a little-endian `u64` field.
    ///
    /// # Panics
    ///
    /// Panics when a caller requests bytes outside a previously validated
    /// operation payload. Reducer callers use only fixed fields for that opcode.
    pub fn u64_at(&self, start: usize) -> u64 {
        u64::from_le_bytes(
            self.payload[start..start + 8]
                .try_into()
                .expect("known marker field has exact width"),
        )
    }

    /// Read an exact 32-byte hash field.
    ///
    /// # Panics
    ///
    /// Panics when a caller requests bytes outside a previously validated
    /// operation payload. Reducer callers use only fixed fields for that opcode.
    pub fn hash_at(&self, start: usize) -> Hash32 {
        Hash32(
            self.payload[start..start + 32]
                .try_into()
                .expect("known marker field has exact width"),
        )
    }

    /// Extract namespace from a complete, defined v1 non-INIT marker.
    pub fn observed_namespace(&self) -> Option<Hash32> {
        (self.version == 1 && self.opcode.is_some_and(|opcode| opcode != Opcode::Init))
            .then(|| self.hash_at(8))
    }
}

/// Stable marker parse failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarkerError {
    /// Stable reason.
    pub reason: Reason,
}

impl MarkerError {
    const fn new(reason: Reason) -> Self {
        Self { reason }
    }
}

/// Detect a broad Tandem marker candidate from raw script bytes.
pub fn find_marker_candidate(script: &[u8], vout: u32) -> Option<MarkerCandidate> {
    if script.first() != Some(&0x6a) {
        return None;
    }
    let opcode = *script.get(1)?;
    let (declared_len, prefix_len, prefix_complete, minimal_push) = match opcode {
        0x01..=0x4b => (usize::from(opcode), 2, true, true),
        0x4c => {
            let length = script.get(2).copied()?;
            (usize::from(length), 3, true, length >= 0x4c)
        }
        0x4d => {
            let bytes = script.get(2..4)?;
            let length = usize::from(u16::from_le_bytes([bytes[0], bytes[1]]));
            (length, 4, true, length > 0xff)
        }
        0x4e => {
            let bytes = script.get(2..6)?;
            let length = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let length = usize::try_from(length).ok()?;
            (length, 6, true, length > 0xffff)
        }
        _ => return None,
    };
    if !prefix_complete || declared_len < 4 {
        return None;
    }
    let available = script.len().saturating_sub(prefix_len);
    if available < 4 || script.get(prefix_len..prefix_len + 4) != Some(&MAGIC) {
        return None;
    }
    let physical_len = declared_len.min(available);
    let payload = script[prefix_len..prefix_len + physical_len].to_vec();
    Some(MarkerCandidate {
        vout,
        declared_len,
        payload,
        prefix_complete,
        payload_complete: available >= declared_len,
        no_trailing_bytes: available == declared_len,
        minimal_push,
    })
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        hex::decode(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broad_candidate_keeps_truncated_payload() {
        let script = hex::decode("6a4c4e544e444d0102").expect("hex");
        let candidate = find_marker_candidate(&script, 7).expect("candidate");
        assert!(!candidate.payload_complete);
        assert_eq!(candidate.vout, 7);
        assert_eq!(
            candidate.parse().expect_err("must fail").reason,
            Reason::BadMarkerEncodingOrLength
        );
    }

    #[test]
    fn foreign_init_is_detected_before_strict_parse() {
        let script = hex::decode("6a07544e444d010300").expect("hex");
        let candidate = find_marker_candidate(&script, 0).expect("candidate");
        assert!(candidate.is_init_candidate());
        assert_eq!(
            candidate
                .parse()
                .expect_err("wrong exact INIT length")
                .reason,
            Reason::BadMarkerEncodingOrLength
        );
    }
}
