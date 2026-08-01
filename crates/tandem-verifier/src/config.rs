//! Fail-closed process configuration.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tandem_core::{Binding, Hash32, Network, wire_hash};
use url::Url;

/// Process arguments. Every consensus binding and release identity is explicit.
#[derive(Clone, Debug, Parser)]
#[command(name = "tandem-verifier", version, about)]
pub struct Args {
    /// Bound Bitcoin network.
    #[arg(long, env = "TANDEM_NETWORK")]
    pub network: String,
    /// Configured INIT txid in Bitcoin display order.
    #[arg(long, env = "TANDEM_INIT_TXID")]
    pub init_txid: String,
    /// Frozen Tandem specification file.
    #[arg(long, env = "TANDEM_SPEC_PATH")]
    pub spec_path: PathBuf,
    /// Expected SHA256 of the exact specification bytes.
    #[arg(long, env = "TANDEM_SPEC_SHA256")]
    pub spec_sha256: String,
    /// `PostgreSQL` connection URL.
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: String,
    /// Bitcoin Core JSON-RPC URL.
    #[arg(long, env = "BITCOIN_RPC_URL")]
    pub bitcoin_rpc_url: String,
    /// Bitcoin Core RPC username.
    #[arg(long, env = "BITCOIN_RPC_USER")]
    pub bitcoin_rpc_user: String,
    /// Bitcoin Core RPC password.
    #[arg(long, env = "BITCOIN_RPC_PASSWORD")]
    pub bitcoin_rpc_password: String,
    /// HTTP listen address.
    #[arg(long, env = "TANDEM_BIND_ADDR", default_value = "127.0.0.1:8088")]
    pub bind_addr: String,
    /// Ed25519 signing seed file. The file must contain exactly 64 hexadecimal characters.
    #[arg(long, env = "TANDEM_SIGNING_KEY_FILE")]
    pub signing_key_file: PathBuf,
    /// Public signer key identifier.
    #[arg(long, env = "TANDEM_SIGNING_KEY_ID")]
    pub signing_key_id: String,
    /// Exact parser source commit.
    #[arg(long, env = "TANDEM_PARSER_COMMIT")]
    pub parser_commit: String,
    /// Exact indexer source commit.
    #[arg(long, env = "TANDEM_INDEXER_COMMIT")]
    pub indexer_commit: String,
    /// SHA256 of the parser release artifact.
    #[arg(long, env = "TANDEM_PARSER_BINARY_SHA256")]
    pub parser_binary_sha256: String,
    /// SHA256 of the indexer release artifact.
    #[arg(long, env = "TANDEM_INDEXER_BINARY_SHA256")]
    pub indexer_binary_sha256: String,
    /// Poll interval when no ZMQ wakeup is configured.
    #[arg(long, env = "TANDEM_POLL_INTERVAL_MS", default_value_t = 5_000)]
    pub poll_interval_ms: u64,
    /// Optional rawblock ZMQ endpoint used only as a wakeup signal.
    #[arg(long, env = "TANDEM_ZMQ_RAWBLOCK_URL")]
    pub zmq_rawblock_url: Option<String>,
    /// Permit mainnet only when a separately hashed authorization artifact also validates.
    #[arg(long, env = "TANDEM_ALLOW_MAINNET", default_value_t = false)]
    pub allow_mainnet: bool,
    /// Mainnet authorization JSON file.
    #[arg(long, env = "TANDEM_MAINNET_AUTHORIZATION_FILE")]
    pub mainnet_authorization_file: Option<PathBuf>,
    /// Expected SHA256 of the mainnet authorization JSON bytes.
    #[arg(long, env = "TANDEM_MAINNET_AUTHORIZATION_SHA256")]
    pub mainnet_authorization_sha256: Option<String>,
}

/// Validated process configuration.
#[derive(Clone)]
pub struct Config {
    /// Immutable protocol binding.
    pub binding: Binding,
    /// `PostgreSQL` URL.
    pub database_url: String,
    /// Bitcoin Core URL.
    pub bitcoin_rpc_url: Url,
    /// Bitcoin Core username.
    pub bitcoin_rpc_user: String,
    /// Bitcoin Core password.
    pub bitcoin_rpc_password: String,
    /// HTTP listen address.
    pub bind_addr: SocketAddr,
    /// Signing seed file.
    pub signing_key_file: PathBuf,
    /// Stable public key identifier.
    pub signing_key_id: String,
    /// Parser commit.
    pub parser_commit: String,
    /// Indexer commit.
    pub indexer_commit: String,
    /// Parser artifact hash.
    pub parser_binary_sha256: Hash32,
    /// Indexer artifact hash.
    pub indexer_binary_sha256: Hash32,
    /// Poll interval.
    pub poll_interval: Duration,
    /// Optional rawblock endpoint.
    pub zmq_rawblock_url: Option<String>,
}

/// Immutable source and binary identity persisted with each processed height.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReleaseIdentity {
    /// Parser source commit.
    pub parser_commit: String,
    /// Indexer source commit.
    pub indexer_commit: String,
    /// Parser release artifact digest.
    pub parser_binary_sha256: Hash32,
    /// Indexer release artifact digest.
    pub indexer_binary_sha256: Hash32,
}

impl Config {
    /// Parse process arguments and verify the specification byte contract.
    pub fn load() -> Result<Self> {
        Self::from_args(Args::parse())
    }

    /// Validate explicit arguments. Useful for tests without process environment mutation.
    pub fn from_args(args: Args) -> Result<Self> {
        let network = Network::from_str(&args.network).map_err(anyhow::Error::msg)?;
        ensure!(
            args.init_txid.len() == 64,
            "configured INIT txid must be 64 hexadecimal characters"
        );
        let init_txid =
            wire_hash(&args.init_txid).context("configured INIT txid is not hexadecimal")?;
        ensure!(!init_txid.is_zero(), "configured INIT txid cannot be zero");

        let expected_spec_hash = Hash32::from_hex(&args.spec_sha256)
            .context("TANDEM_SPEC_SHA256 must be 64 hexadecimal characters")?;
        let spec = std::fs::read(&args.spec_path).with_context(|| {
            format!("cannot read specification at {}", args.spec_path.display())
        })?;
        verify_spec_bytes(&spec)?;
        let actual_spec_hash = Hash32(Sha256::digest(&spec).into());
        ensure!(
            actual_spec_hash == expected_spec_hash,
            "specification SHA256 mismatch: expected {expected_spec_hash}, got {actual_spec_hash}"
        );

        ensure!(
            args.database_url.starts_with("postgres://")
                || args.database_url.starts_with("postgresql://"),
            "DATABASE_URL must be PostgreSQL"
        );
        let bitcoin_rpc_url =
            Url::parse(&args.bitcoin_rpc_url).context("invalid Bitcoin Core RPC URL")?;
        ensure!(
            matches!(bitcoin_rpc_url.scheme(), "http" | "https"),
            "Bitcoin Core RPC URL must use HTTP or HTTPS"
        );
        let bind_addr = args
            .bind_addr
            .parse()
            .context("invalid HTTP bind address")?;
        validate_key_id(&args.signing_key_id)?;
        validate_commit("TANDEM_PARSER_COMMIT", &args.parser_commit)?;
        validate_commit("TANDEM_INDEXER_COMMIT", &args.indexer_commit)?;
        let parser_binary_sha256 = Hash32::from_hex(&args.parser_binary_sha256)
            .context("invalid TANDEM_PARSER_BINARY_SHA256")?;
        let indexer_binary_sha256 = Hash32::from_hex(&args.indexer_binary_sha256)
            .context("invalid TANDEM_INDEXER_BINARY_SHA256")?;
        ensure!(
            args.poll_interval_ms >= 250,
            "poll interval must be at least 250 ms"
        );
        if args.bitcoin_rpc_user.is_empty() || args.bitcoin_rpc_password.is_empty() {
            bail!("Bitcoin Core RPC credentials cannot be empty");
        }

        let binding = Binding {
            network,
            init_txid,
            spec_hash: actual_spec_hash,
        };
        validate_mainnet_gate(&args, &binding)?;

        Ok(Self {
            binding,
            database_url: args.database_url,
            bitcoin_rpc_url,
            bitcoin_rpc_user: args.bitcoin_rpc_user,
            bitcoin_rpc_password: args.bitcoin_rpc_password,
            bind_addr,
            signing_key_file: args.signing_key_file,
            signing_key_id: args.signing_key_id,
            parser_commit: args.parser_commit,
            indexer_commit: args.indexer_commit,
            parser_binary_sha256,
            indexer_binary_sha256,
            poll_interval: Duration::from_millis(args.poll_interval_ms),
            zmq_rawblock_url: args.zmq_rawblock_url,
        })
    }

    /// Return the release identity that must be stored with every applied block.
    pub fn release_identity(&self) -> ReleaseIdentity {
        ReleaseIdentity {
            parser_commit: self.parser_commit.clone(),
            indexer_commit: self.indexer_commit.clone(),
            parser_binary_sha256: self.parser_binary_sha256,
            indexer_binary_sha256: self.indexer_binary_sha256,
        }
    }
}

fn verify_spec_bytes(bytes: &[u8]) -> Result<()> {
    ensure!(
        !bytes.starts_with(&[0xef, 0xbb, 0xbf]),
        "specification must not contain a UTF-8 BOM"
    );
    ensure!(
        std::str::from_utf8(bytes).is_ok(),
        "specification must be valid UTF-8"
    );
    ensure!(
        !bytes.contains(&b'\r'),
        "specification must use LF line endings"
    );
    ensure!(
        bytes.last() == Some(&b'\n'),
        "specification must end in exactly one LF"
    );
    ensure!(
        !bytes.ends_with(b"\n\n"),
        "specification must end in exactly one LF"
    );
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.last().is_some_and(|byte| matches!(byte, b' ' | b'\t')) {
            bail!(
                "specification line {} has trailing horizontal whitespace",
                index + 1
            );
        }
    }
    Ok(())
}

fn validate_key_id(value: &str) -> Result<()> {
    ensure!(
        (1..=128).contains(&value.len()),
        "signing key id length must be 1 through 128"
    );
    ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte)),
        "signing key id contains an unsupported character"
    );
    Ok(())
}

fn validate_commit(name: &str, value: &str) -> Result<()> {
    ensure!(
        value.len() == 40
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "{name} must be 40 lowercase hexadecimal characters"
    );
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct MainnetAuthorization {
    schema: String,
    protocol_id: String,
    spec_hash: Hash32,
    approved_by: Vec<String>,
    approved_at: String,
}

fn validate_mainnet_gate(args: &Args, binding: &Binding) -> Result<()> {
    if binding.network != Network::Mainnet {
        return Ok(());
    }
    ensure!(
        args.allow_mainnet,
        "mainnet requires TANDEM_ALLOW_MAINNET=true"
    );
    let path = args
        .mainnet_authorization_file
        .as_ref()
        .context("mainnet requires TANDEM_MAINNET_AUTHORIZATION_FILE")?;
    let expected = args
        .mainnet_authorization_sha256
        .as_deref()
        .context("mainnet requires TANDEM_MAINNET_AUTHORIZATION_SHA256")?;
    let expected = Hash32::from_hex(expected).context("invalid mainnet authorization SHA256")?;
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read mainnet authorization at {}", path.display()))?;
    ensure!(
        Hash32::sha256(&bytes) == expected,
        "mainnet authorization SHA256 mismatch"
    );
    let authorization: MainnetAuthorization =
        serde_json::from_slice(&bytes).context("invalid mainnet authorization JSON")?;
    ensure!(
        authorization.schema == "urn:tandem:mainnet-authorization:v1",
        "wrong mainnet authorization schema"
    );
    ensure!(
        authorization.protocol_id == binding.protocol_id(),
        "mainnet authorization protocol id mismatch"
    );
    ensure!(
        authorization.spec_hash == binding.spec_hash,
        "mainnet authorization spec hash mismatch"
    );
    let named_approvers = authorization
        .approved_by
        .iter()
        .map(|approver| approver.trim())
        .filter(|approver| !approver.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    ensure!(
        named_approvers.len() >= 2,
        "mainnet authorization requires two named approvers"
    );
    ensure!(
        !authorization.approved_at.trim().is_empty(),
        "mainnet authorization approval time is missing"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_crlf_spec() {
        assert!(verify_spec_bytes(b"abc\r\n").is_err());
    }

    #[test]
    fn accepts_exact_lf_contract() {
        assert!(verify_spec_bytes(b"abc\n").is_ok());
    }

    #[test]
    fn mainnet_gate_rejects_implicit_activation() {
        let args = Args {
            network: "mainnet".to_owned(),
            init_txid: "01".repeat(32),
            spec_path: PathBuf::from("unused"),
            spec_sha256: "02".repeat(32),
            database_url: "postgresql://unused".to_owned(),
            bitcoin_rpc_url: "http://127.0.0.1:8332".to_owned(),
            bitcoin_rpc_user: "unused".to_owned(),
            bitcoin_rpc_password: "unused".to_owned(),
            bind_addr: "127.0.0.1:8088".to_owned(),
            signing_key_file: PathBuf::from("unused"),
            signing_key_id: "unused".to_owned(),
            parser_commit: "a".repeat(40),
            indexer_commit: "b".repeat(40),
            parser_binary_sha256: "c".repeat(64),
            indexer_binary_sha256: "d".repeat(64),
            poll_interval_ms: 5_000,
            zmq_rawblock_url: None,
            allow_mainnet: false,
            mainnet_authorization_file: None,
            mainnet_authorization_sha256: None,
        };
        let binding = Binding {
            network: Network::Mainnet,
            init_txid: Hash32([1; 32]),
            spec_hash: Hash32([2; 32]),
        };
        assert!(
            validate_mainnet_gate(&args, &binding)
                .expect_err("must reject")
                .to_string()
                .contains("TANDEM_ALLOW_MAINNET")
        );
    }
}
