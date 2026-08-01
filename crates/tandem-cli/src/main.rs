//! Independent Tandem golden-vector verifier and resolved-block replay CLI.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tandem_core::{
    Binding, BlockView, ChainState, Hash32, Key33, KeyPair, Network, Opcode, apply_block,
    block_root, find_marker_candidate, object_key, wire_hash,
};

#[derive(Parser)]
#[command(
    name = "tandem",
    version,
    about = "Independent Tandem verification tools"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Verify the shared golden corpus without importing another implementation.
    VerifyVectors {
        /// Golden vector manifest path.
        #[arg(long)]
        manifest: PathBuf,
        /// Exact normative specification path.
        #[arg(long)]
        spec: PathBuf,
    },
    /// Verify every frozen shared input listed by the Rust input lock.
    VerifyInputs {
        /// Pipeline B input lock file.
        #[arg(long)]
        lock: PathBuf,
        /// Root of the checked-out Tandem protocol repository.
        #[arg(long)]
        protocol_root: PathBuf,
    },
    /// Replay resolved JSON block views through the independent reducer.
    Replay {
        /// Replay file containing a binding, blocks, and optional expected roots.
        #[arg(long)]
        input: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::VerifyVectors { manifest, spec } => {
            let report = verify_vectors(&manifest, &spec)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::VerifyInputs {
            lock,
            protocol_root,
        } => {
            let report = verify_inputs(&lock, &protocol_root)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Replay { input } => {
            let report = replay(&input)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct InputLock {
    schema: String,
    inputs: BTreeMap<String, Hash32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InputReport {
    schema: &'static str,
    files_verified: usize,
    hashes: BTreeMap<String, Hash32>,
}

fn verify_inputs(lock_path: &Path, protocol_root: &Path) -> Result<InputReport> {
    let bytes = fs::read(lock_path).context("cannot read protocol input lock")?;
    let lock: InputLock = serde_json::from_slice(&bytes).context("invalid protocol input lock")?;
    ensure!(
        lock.schema == "urn:tandem:rust-protocol-input-lock",
        "wrong protocol input lock schema"
    );
    ensure!(!lock.inputs.is_empty(), "protocol input lock is empty");
    let mut hashes = BTreeMap::new();
    for (relative, expected) in &lock.inputs {
        let path = Path::new(relative);
        ensure!(
            !path.is_absolute()
                && path
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))),
            "unsafe protocol input path {relative}"
        );
        let content = fs::read(protocol_root.join(path))
            .with_context(|| format!("cannot read protocol input {relative}"))?;
        let actual = Hash32(Sha256::digest(content).into());
        ensure!(
            actual == *expected,
            "protocol input hash mismatch for {relative}"
        );
        hashes.insert(relative.clone(), actual);
    }
    Ok(InputReport {
        schema: "urn:tandem:rust-input-verification-report",
        files_verified: hashes.len(),
        hashes,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema: String,
    specification: String,
    spec_hash: Hash32,
    fixture_file: String,
    fixture_sha256: Hash32,
    vector_root: Hash32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VectorReport {
    schema: &'static str,
    spec_hash: Hash32,
    fixture_sha256: Hash32,
    vector_root: Hash32,
    markers_verified: usize,
    event_leaves_verified: usize,
    object_leaves_verified: usize,
    event_root: Hash32,
    object_state_root: Hash32,
    chained_root: Hash32,
}

fn verify_vectors(manifest_path: &Path, spec_path: &Path) -> Result<VectorReport> {
    let manifest_bytes = fs::read(manifest_path).context("cannot read vector manifest")?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).context("invalid vector manifest")?;
    ensure!(
        manifest.schema == "urn:tandem:golden-vector-manifest",
        "wrong vector manifest schema"
    );
    ensure!(
        manifest.specification == "tandem.md",
        "unexpected specification filename"
    );
    ensure!(
        manifest.fixture_file == "golden.json",
        "unexpected fixture filename"
    );

    let spec_bytes = fs::read(spec_path).context("cannot read specification")?;
    let spec_hash = Hash32(Sha256::digest(&spec_bytes).into());
    ensure!(
        spec_hash == manifest.spec_hash,
        "specification digest mismatch"
    );

    let fixture_path = manifest_path
        .parent()
        .context("manifest has no parent directory")?
        .join(&manifest.fixture_file);
    let fixture_bytes = fs::read(&fixture_path).context("cannot read fixture file")?;
    let fixture_hash = Hash32(Sha256::digest(&fixture_bytes).into());
    ensure!(
        fixture_hash == manifest.fixture_sha256,
        "fixture digest mismatch"
    );
    let vector_root = vector_root(&manifest.fixture_file, fixture_hash);
    ensure!(vector_root == manifest.vector_root, "vector root mismatch");

    let fixture: Value = serde_json::from_slice(&fixture_bytes).context("invalid fixture JSON")?;
    ensure!(
        fixture["schema"] == "urn:tandem:golden-fixtures",
        "wrong fixture schema"
    );
    ensure!(
        fixture["specHash"] == spec_hash.to_hex(),
        "fixture spec hash mismatch"
    );
    verify_identity(&fixture, spec_hash)?;
    let markers_verified = verify_markers(&fixture)?;
    verify_carrier(&fixture)?;
    let roots = &fixture["roots"];
    let event_leaves_verified = verify_preimage_leaves(&roots["events"])?;
    let object_leaves_verified = verify_preimage_leaves(&roots["snapshots"])?;
    let event_root = verify_merkle_levels(
        &roots["eventMerkleLevels"],
        b"TANDEM/EVENT-NODE\0",
        parse_json_hash(&roots["eventRoot"])?,
    )?;
    let object_state_root = verify_merkle_levels(
        &roots["objectMerkleLevels"],
        b"TANDEM/OBJECT-NODE\0",
        parse_json_hash(&roots["objectStateRoot"])?,
    )?;
    let chained_root = verify_block_root(&fixture, event_root, object_state_root)?;

    Ok(VectorReport {
        schema: "urn:tandem:rust-vector-report",
        spec_hash,
        fixture_sha256: fixture_hash,
        vector_root,
        markers_verified,
        event_leaves_verified,
        object_leaves_verified,
        event_root,
        object_state_root,
        chained_root,
    })
}

fn vector_root(filename: &str, fixture_hash: Hash32) -> Hash32 {
    let mut preimage = Vec::with_capacity(19 + filename.len() + 1 + 32);
    preimage.extend_from_slice(b"TANDEM/VECTOR-ROOT\0");
    preimage.extend_from_slice(filename.as_bytes());
    preimage.push(0);
    preimage.extend_from_slice(&fixture_hash.0);
    Hash32::sha256(preimage)
}

fn verify_identity(fixture: &Value, spec_hash: Hash32) -> Result<()> {
    let protocol_id = fixture["identity"]["protocolId"]
        .as_str()
        .context("missing protocol id")?;
    let protocol_parts = protocol_id.split(':').collect::<Vec<_>>();
    ensure!(
        protocol_parts.len() == 3 && protocol_parts[..2] == ["tndm", "regtest"],
        "invalid protocol id"
    );
    let binding = Binding {
        network: Network::Regtest,
        init_txid: wire_hash(protocol_parts[2]).context("invalid INIT txid")?,
        spec_hash,
    };
    ensure!(
        binding.protocol_id() == protocol_id,
        "protocol id derivation mismatch"
    );
    let expected_namespace = parse_json_hash(&fixture["identity"]["namespace"])?;
    ensure!(
        binding.namespace() == expected_namespace,
        "namespace derivation mismatch"
    );

    let object_display_id = fixture["identity"]["objectDisplayId"]
        .as_str()
        .context("missing object display id")?;
    let object_parts = object_display_id.split(':').collect::<Vec<_>>();
    ensure!(
        object_parts.len() == 5
            && object_parts[..2] == ["tandem", "regtest"]
            && object_parts[2] == protocol_parts[2]
            && object_parts[4] == "1",
        "invalid object display id"
    );
    let create_txid = object_parts[3];
    let expected_object = parse_json_hash(&fixture["identity"]["objectKey"])?;
    ensure!(
        object_key(expected_namespace, wire_hash(create_txid)?) == expected_object,
        "object key derivation mismatch"
    );
    Ok(())
}

fn verify_markers(fixture: &Value) -> Result<usize> {
    let markers = fixture["markers"]
        .as_array()
        .context("missing marker vectors")?;
    for marker in markers {
        let script = hex::decode(
            marker["scriptHex"]
                .as_str()
                .context("missing marker script")?,
        )?;
        let candidate = find_marker_candidate(&script, 0).context("marker was not detected")?;
        let parsed = candidate
            .parse()
            .map_err(|error| anyhow::anyhow!("marker parse failed: {:?}", error.reason))?;
        let expected_opcode = match marker["operation"]
            .as_str()
            .context("missing marker operation")?
        {
            "INIT" => Opcode::Init,
            "CREATE" => Opcode::Create,
            "MARK" => Opcode::Mark,
            "ROTATE" => Opcode::Rotate,
            "CLOSE" => Opcode::Close,
            value => bail!("unknown marker operation {value}"),
        };
        ensure!(
            parsed.opcode == Some(expected_opcode),
            "marker opcode mismatch"
        );
        ensure!(
            parsed.payload.len() == expected_opcode.payload_len(),
            "marker payload length mismatch"
        );
        ensure!(
            script.len()
                == usize::try_from(
                    marker["scriptBytes"]
                        .as_u64()
                        .context("missing script size")?
                )?,
            "marker script size mismatch"
        );
        ensure!(
            parsed.payload.len()
                == usize::try_from(
                    marker["payloadBytes"]
                        .as_u64()
                        .context("missing payload size")?
                )?,
            "marker payload size mismatch"
        );
    }
    Ok(markers.len())
}

fn verify_carrier(fixture: &Value) -> Result<()> {
    let carrier = &fixture["carrier"];
    let pair = KeyPair::checked(
        Key33::from_hex(carrier["key0"].as_str().context("missing key0")?)?,
        Key33::from_hex(carrier["key1"].as_str().context("missing key1")?)?,
    )
    .context("invalid carrier key pair")?;
    ensure!(
        hex::encode(pair.witness_script()) == carrier["witnessScript"],
        "witness script mismatch"
    );
    ensure!(
        hex::encode(pair.carrier_script()) == carrier["scriptPubKey"],
        "carrier scriptPubKey mismatch"
    );
    Ok(())
}

fn verify_preimage_leaves(value: &Value) -> Result<usize> {
    let leaves = value.as_array().context("missing root leaves")?;
    for leaf in leaves {
        let preimage = hex::decode(leaf["preimageHex"].as_str().context("missing preimage")?)?;
        let expected = parse_json_hash(&leaf["leafHex"])?;
        ensure!(Hash32::sha256(preimage) == expected, "leaf digest mismatch");
    }
    Ok(leaves.len())
}

fn verify_merkle_levels(value: &Value, domain: &[u8], expected_root: Hash32) -> Result<Hash32> {
    let levels = value.as_array().context("missing Merkle levels")?;
    ensure!(!levels.is_empty(), "empty Merkle level list");
    for pair in levels.windows(2) {
        let mut children = parse_hash_array(&pair[0])?;
        let parents = parse_hash_array(&pair[1])?;
        if children.len() % 2 == 1 {
            children.push(*children.last().context("empty child level")?);
        }
        ensure!(
            parents.len() == children.len() / 2,
            "Merkle level width mismatch"
        );
        for (index, child_pair) in children.chunks_exact(2).enumerate() {
            let mut preimage = Vec::with_capacity(domain.len() + 64);
            preimage.extend_from_slice(domain);
            preimage.extend_from_slice(&child_pair[0].0);
            preimage.extend_from_slice(&child_pair[1].0);
            ensure!(
                Hash32::sha256(preimage) == parents[index],
                "Merkle parent mismatch"
            );
        }
    }
    let root = *parse_hash_array(levels.last().context("missing root level")?)?
        .first()
        .context("root level is empty")?;
    ensure!(root == expected_root, "reported Merkle root mismatch");
    Ok(root)
}

fn verify_block_root(fixture: &Value, event_root: Hash32, object_root: Hash32) -> Result<Hash32> {
    let namespace = parse_json_hash(&fixture["identity"]["namespace"])?;
    let initial_root = parse_json_hash(&fixture["roots"]["initialStateRoot"])?;
    let mut initial_preimage = Vec::with_capacity(19 + 32);
    initial_preimage.extend_from_slice(b"TANDEM/STATE-EMPTY\0");
    initial_preimage.extend_from_slice(&namespace.0);
    ensure!(
        Hash32::sha256(initial_preimage) == initial_root,
        "initial state root mismatch"
    );

    let first_event_preimage = hex::decode(
        fixture["roots"]["events"][0]["preimageHex"]
            .as_str()
            .context("missing event preimage")?,
    )?;
    let event_domain_len = b"TANDEM/EVENT\0".len();
    let block_hash =
        Hash32(first_event_preimage[event_domain_len + 32..event_domain_len + 64].try_into()?);
    let height = u64::from_le_bytes(
        first_event_preimage[event_domain_len + 64..event_domain_len + 72].try_into()?,
    );
    let snapshots = fixture["roots"]["snapshots"]
        .as_array()
        .context("missing snapshots")?;
    let object_domain_len = b"TANDEM/OBJECT-STATE\0".len();
    let mut founding = 0_u64;
    let mut active = 0_u64;
    for snapshot in snapshots {
        let preimage = hex::decode(
            snapshot["preimageHex"]
                .as_str()
                .context("missing snapshot preimage")?,
        )?;
        founding += u64::from(preimage[object_domain_len + 32]);
        active += u64::from(preimage[object_domain_len + 33] == 0);
    }
    let calculated = block_root(
        namespace,
        initial_root,
        block_hash,
        height,
        event_root,
        object_root,
        tandem_core::Counters {
            founding_created: founding,
            all_objects: snapshots.len() as u64,
            active_objects: active,
        },
    );
    let expected = parse_json_hash(&fixture["roots"]["chainedBlockRoot"])?;
    ensure!(calculated == expected, "chained block root mismatch");
    Ok(calculated)
}

fn parse_json_hash(value: &Value) -> Result<Hash32> {
    Hash32::from_hex(value.as_str().context("expected hexadecimal hash")?).context("invalid hash")
}

fn parse_hash_array(value: &Value) -> Result<Vec<Hash32>> {
    value
        .as_array()
        .context("expected hash array")?
        .iter()
        .map(parse_json_hash)
        .collect()
}

#[derive(Deserialize)]
struct ReplayInput {
    binding: Binding,
    blocks: Vec<BlockView>,
    #[serde(default)]
    expected_roots: Vec<tandem_core::HeightRoots>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplayReport {
    schema: &'static str,
    protocol_id: String,
    heights: Vec<tandem_core::HeightRoots>,
    event_counts: Vec<usize>,
    final_state: ChainState,
}

fn replay(path: &Path) -> Result<ReplayReport> {
    let bytes = fs::read(path).context("cannot read replay input")?;
    let input: ReplayInput = serde_json::from_slice(&bytes).context("invalid replay JSON")?;
    let mut state = ChainState::new(input.binding);
    let mut heights = Vec::with_capacity(input.blocks.len());
    let mut event_counts = Vec::with_capacity(input.blocks.len());
    for (index, block) in input.blocks.iter().enumerate() {
        let delta = apply_block(&mut state, block)
            .with_context(|| format!("replay failed at input block {index}"))?;
        if let Some(expected) = input.expected_roots.get(index) {
            ensure!(
                &delta.roots == expected,
                "root mismatch at height {}",
                block.height
            );
        }
        event_counts.push(delta.events.len());
        heights.push(delta.roots);
    }
    ensure!(
        input.expected_roots.is_empty() || input.expected_roots.len() == heights.len(),
        "expected root list length differs from block list"
    );
    Ok(ReplayReport {
        schema: "urn:tandem:rust-replay-report",
        protocol_id: state.binding.protocol_id(),
        heights,
        event_counts,
        final_state: state,
    })
}
