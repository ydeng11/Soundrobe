//! Stable, public proposals and explicit owner intent. No dispatch authority.

use super::{digest_valid, policy, require, sha};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

const MAX_RECORD: usize = 1024 * 1024;
const MAX_INTENT: usize = 8192;
const MAX_LIFETIME: u64 = 1800;
const INTENT_DOMAIN: &[u8] = b"soundrobe/owner-execution-intent/v1\0";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ExecutionKind {
    OfflineRehearsal,
    HostedSingleRequest,
}

/// Public, reviewable data; possession is not consent or permission to dispatch.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExecutionProposal {
    pub(crate) version: u8,
    pub(crate) proposal_id: String,
    pub(crate) execution_kind: ExecutionKind,
    pub(crate) executable_sha256: String,
    pub(crate) membership_sha256: String,
    pub(crate) native_plan_sha256: String,
    pub(crate) request_sha256: String,
    pub(crate) verifier_sha256: String,
    pub(crate) boundary_pricing_sha256: String,
    pub(crate) context_sha256: String,
    pub(crate) sources_and_tools: Value,
}

fn canonical(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, String> {
    // Value maps use serde_json's existing sorted map representation, including
    // nested structs. Do not sign declaration-order struct serialization.
    let value = serde_json::to_value(value).map_err(|_| "owner_record_invalid")?;
    let bytes = serde_json::to_vec(&value).map_err(|_| "owner_record_invalid")?;
    require(bytes.len() <= limit)?;
    Ok(bytes)
}

fn uuid_valid(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .is_ok_and(|id| id.get_version_num() == 4 && id.to_string() == value)
}

fn source_path_valid(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.chars().any(char::is_control)
        && !Path::new(path).is_absolute()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn runtime_marker(path: &str) -> Value {
    json!({"path":path,"runtime":"nativeSupervision"})
}

/// Preserve every lock and its position. Only the exact planned run-root slot
/// changes representation: its unpredictable digest becomes a typed marker.
fn normalized_sources(bindings: &Value, frozen: bool) -> Result<(Value, Option<String>), String> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("owner_sources_invalid")?;
    let runtime_path = match bindings.get("disposableRunRoot") {
        None => None,
        Some(root) => {
            let root = Path::new(root.as_str().ok_or("owner_runtime_path_invalid")?);
            require(
                root.is_absolute()
                    && root.starts_with(workspace.join(".amp/in/artifacts"))
                    && root != workspace.join(".amp/in/artifacts"),
            )?;
            let relative = root
                .join("native-supervision.json")
                .strip_prefix(workspace)
                .map_err(|_| "owner_runtime_path_invalid")?
                .to_string_lossy()
                .into_owned();
            require(source_path_valid(&relative))?;
            Some(relative)
        }
    };
    let sources = bindings["sourcesAndTools"]
        .as_array()
        .ok_or("owner_sources_invalid")?;
    require(!sources.is_empty() && sources.len() <= 4096)?;
    let mut normalized = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut runtime_count = 0;
    let mut runtime_sha = None;
    for source in sources {
        let path = source["path"].as_str().ok_or("owner_sources_invalid")?;
        require(
            source_path_valid(path)
                && seen.insert(path)
                && source.as_object().is_some_and(|o| o.len() == 2),
        )?;
        if runtime_path.as_deref() == Some(path) {
            runtime_count += 1;
            if source == &runtime_marker(path) {
                require(!frozen)?;
            } else {
                let digest = source["sha256"]
                    .as_str()
                    .ok_or("owner_runtime_digest_invalid")?;
                require(digest_valid(digest))?;
                runtime_sha = Some(digest.into());
            }
            normalized.push(runtime_marker(path));
        } else {
            require(
                Path::new(path)
                    .file_name()
                    .is_none_or(|n| n != "native-supervision.json")
                    && source["sha256"].as_str().is_some_and(digest_valid),
            )?;
            normalized.push(source.clone());
        }
    }
    require(
        runtime_count == usize::from(runtime_path.is_some())
            && (bindings.get("supervision").is_none() || runtime_path.is_some()),
    )?;
    Ok((Value::Array(normalized), runtime_sha))
}

fn validate_runtime_supervision(bindings: &Value) -> Result<(), String> {
    let (_, runtime_sha) = normalized_sources(bindings, true)?;
    let Some(expected_sha) = runtime_sha else {
        return Ok(());
    };
    // Native supervision adds only executionGroup after reading the immutable
    // receiver file. Reconstruct that exact canonical file, not an arbitrary
    // projection that could discard an attacker-selected changed field.
    let mut config = bindings["supervision"].clone();
    let object = config
        .as_object_mut()
        .ok_or("owner_runtime_supervision_invalid")?;
    let group = object
        .remove("executionGroup")
        .and_then(|v| v.as_i64())
        .ok_or("owner_runtime_supervision_invalid")?;
    let expected_keys: std::collections::BTreeSet<_> = [
        "generation",
        "start",
        "preparationMs",
        "executionMs",
        "receiverPid",
        "receiverGroup",
        "control",
        "storage",
    ]
    .into_iter()
    .collect();
    require(
        object
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>()
            == expected_keys,
    )?;
    let generation = config["generation"]
        .as_str()
        .ok_or("owner_runtime_supervision_invalid")?;
    let receiver_group = config["receiverGroup"]
        .as_i64()
        .ok_or("owner_runtime_supervision_invalid")?;
    require(
        uuid_valid(generation)
            && config["start"].as_u64().is_some_and(|v| v > 0)
            && config["preparationMs"] == 300_000
            && config["executionMs"] == 600_000
            && config["receiverPid"]
                .as_u64()
                .is_some_and(|v| v > 1 && v <= i32::MAX as u64)
            && group > 1
            && group <= i32::MAX as i64
            && receiver_group > 1
            && receiver_group <= i32::MAX as i64
            && group != receiver_group
            && config["control"] == "",
    )?;
    let mib = 1024 * 1024u64;
    require(
        config["storage"]
            == json!({"generation":generation,"logicalLimit":512*mib,
        "applicationWriteLimit":1024*mib,"terminalGrowthReserved":2*mib,"terminalWritesReserved":8*mib,
        "selfGrowthConsumed":2*mib,"selfWritesConsumed":4*mib,"maximumOwners":255,
        "failedBookkeepingReserved":128*1024,"launcherTerminalPartition":64*1024,
        "coverage":"evaluation_files_sqlite_helpers_prepaid_not_physical_io"})
            && expected_sha == sha(&canonical(&config, MAX_RECORD)?),
    )
}

fn verifier_digest_valid(pin: &str) -> Result<(), String> {
    require(digest_valid(pin) && pin != sha(&[0; 32]))?;
    // Same three public fixture roots rejected by ApprovalVerifier::read.
    let rfc_fixture = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ];
    require(pin != sha(&rfc_fixture))?;
    for seed in [42, 43] {
        let key = Ed25519KeyPair::from_seed_unchecked(&[seed; 32])
            .map_err(|_| "owner_verifier_invalid")?;
        require(pin != sha(key.public_key().as_ref()))?;
    }
    Ok(())
}

pub(crate) fn current_executable_sha256() -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let path = std::env::current_exe().map_err(|_| "owner_executable_unavailable")?;
    let file = std::fs::File::open(path).map_err(|_| "owner_executable_unavailable")?;
    require(
        file.metadata()
            .is_ok_and(|m| m.is_file() && m.len() <= 512 * 1024 * 1024),
    )?;
    let mut file = file.take(512 * 1024 * 1024 + 1);
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    let mut size = 0u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "owner_executable_unavailable")?;
        if count == 0 {
            break;
        }
        size += count as u64;
        require(size <= 512 * 1024 * 1024)?;
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

impl ExecutionProposal {
    /// Before launch, sourcesAndTools contains all exact static locks plus one
    /// {path:"<repo-relative planned run>/native-supervision.json",runtime:"nativeSupervision"}.
    /// disposableRunRoot must declare that exact absolute run directory. At
    /// freeze the same slot must instead contain {path,sha256} for the native
    /// canonical receiver file. No other source path/digest/order is normalized.
    pub(crate) fn construct(
        kind: ExecutionKind,
        proposal_id: &str,
        membership: &Value,
        bindings: &Value,
        plan: &policy::RehearsalPlan,
        context: &policy::Rehearsal,
        separately_pinned_verifier_sha256: &str,
    ) -> Result<Self, String> {
        let binding = context.binding();
        let plan_value = serde_json::to_value(plan).map_err(|_| "owner_plan_invalid")?;
        let input = serde_json::from_value(plan_value["input"].clone())
            .map_err(|_| "owner_plan_invalid")?;
        let executable = current_executable_sha256()?;
        require(
            bindings["executable"] == executable
                && bindings["modelRehearsalPlan"] == plan_value
                && binding["verifierSha256"] == separately_pinned_verifier_sha256
                && binding["requestSha256"] == sha(&context.request_body(&input)?)
                && binding["requestSha256"] == sha(&canonical(&plan.body(&input)?, MAX_RECORD)?)
                && binding["model"] == plan_value["model"]
                && binding["observedTarget"] == plan_value["observedTarget"]
                && binding["route"] == plan_value["route"]
                && binding["providerClaim"] == plan_value["provider"]
                && binding["liveAuthority"] == false,
        )?;
        require(match kind {
            ExecutionKind::HostedSingleRequest => {
                plan.native_prepared
                    && plan.offline_hosted.is_none()
                    && !plan.offline_http
                    && plan.offline_response.is_none()
                    && context.disabled_hosted()
                    && ["offlineFixture", "offlineHttp", "offlineResponse"]
                        .iter()
                        .all(|k| binding.get(*k).is_none())
            }
            // Native-prepared trust may also be rehearsed entirely offline.
            // The independently selected executor kind, not this preparation
            // marker, determines which intent the caller must verify.
            ExecutionKind::OfflineRehearsal => true,
        })?;
        // The compiled native six-job membership is an exact public contract.
        let expected: Value = serde_json::from_str(include_str!(
            "../../../test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json"
        ))
        .map_err(|_| "owner_membership_invalid")?;
        require(membership == &expected)?;
        if kind == ExecutionKind::HostedSingleRequest {
            let boundary = &binding["boundary"];
            let estimate = boundary["textEstimatePicoUsd"]
                .as_u64()
                .ok_or("owner_estimate_invalid")?;
            require(
                binding["policy"] == policy::boundary::DISABLED_POLICY
                    && boundary["endpoint"] == policy::boundary::ENDPOINT
                    && boundary["allowanceUsdCents"] == 500
                    && estimate < 5_000_000_000_000
                    && boundary["hardBillingCap"] == false
                    && boundary["billingRiskAccepted"] == false
                    && boundary["liveDispatchAvailable"] == false
                    && boundary["readyForExecution"] == false,
            )?;
        }
        let proposal = Self {
            version: 1,
            proposal_id: proposal_id.into(),
            execution_kind: kind,
            executable_sha256: executable,
            membership_sha256: sha(&canonical(membership, MAX_RECORD)?),
            native_plan_sha256: sha(&canonical(plan, MAX_RECORD)?),
            request_sha256: binding["requestSha256"]
                .as_str()
                .ok_or("owner_request_invalid")?
                .into(),
            verifier_sha256: separately_pinned_verifier_sha256.into(),
            boundary_pricing_sha256: sha(&canonical(&binding["boundary"], MAX_RECORD)?),
            context_sha256: sha(&canonical(&binding, MAX_RECORD)?),
            sources_and_tools: normalized_sources(bindings, false)?.0,
        };
        proposal.sha256()?;
        Ok(proposal)
    }

    pub(crate) fn sha256(&self) -> Result<String, String> {
        require(self.version == 1 && uuid_valid(&self.proposal_id))?;
        for digest in [
            &self.executable_sha256,
            &self.membership_sha256,
            &self.native_plan_sha256,
            &self.request_sha256,
            &self.verifier_sha256,
            &self.boundary_pricing_sha256,
            &self.context_sha256,
        ] {
            require(digest_valid(digest))?;
        }
        verifier_digest_valid(&self.verifier_sha256)?;
        let sources = self
            .sources_and_tools
            .as_array()
            .ok_or("owner_sources_invalid")?;
        require(!sources.is_empty() && sources.len() <= 4096)?;
        let mut paths = std::collections::BTreeSet::new();
        let mut runtime_count = 0;
        for source in sources {
            let path = source["path"].as_str().ok_or("owner_sources_invalid")?;
            require(
                source_path_valid(path)
                    && paths.insert(path)
                    && source.as_object().is_some_and(|o| o.len() == 2),
            )?;
            if source == &runtime_marker(path) {
                runtime_count += 1;
                require(
                    Path::new(path)
                        .file_name()
                        .is_some_and(|n| n == "native-supervision.json"),
                )?;
            } else {
                require(
                    Path::new(path)
                        .file_name()
                        .is_none_or(|n| n != "native-supervision.json")
                        && source["sha256"].as_str().is_some_and(digest_valid),
                )?;
            }
        }
        require(runtime_count <= 1)?;
        Ok(sha(&canonical(self, MAX_RECORD)?))
    }

    /// Recheck native public inputs before the preparation clock starts.
    pub(crate) fn verify_inputs(
        &self,
        expected_kind: ExecutionKind,
        membership: &Value,
        bindings: &Value,
        plan: &policy::RehearsalPlan,
        context: &policy::Rehearsal,
        separately_pinned_verifier_sha256: &str,
    ) -> Result<(), String> {
        require(
            self.execution_kind == expected_kind
                && self.verifier_sha256 == separately_pinned_verifier_sha256,
        )?;
        let actual = Self::construct(
            expected_kind,
            &self.proposal_id,
            membership,
            bindings,
            plan,
            context,
            separately_pinned_verifier_sha256,
        )?;
        require(self.sha256()? == actual.sha256()?)
    }

    /// `report` must come directly from the owner's Journal::report, not IPC.
    /// This checks the frozen scope, but never signs the later run approval.
    pub(crate) fn validate_frozen_report(
        &self,
        report: &Value,
        plan: &policy::RehearsalPlan,
        context: &policy::Rehearsal,
    ) -> Result<(), String> {
        validate_runtime_supervision(&report["bindings"])?;
        self.verify_inputs(
            self.execution_kind,
            &report["membership"],
            &report["bindings"],
            plan,
            context,
            &self.verifier_sha256,
        )?;
        require(
            report["state"] == "frozen"
                && report["ownerLive"] == true
                && report["consumptions"] == 0
                && report["credentialLookups"] == 0
                && report["liveAuthority"] == false
                && report["bindings"]["modelRehearsal"] == context.binding()
                && report["runId"].as_str().is_some_and(uuid_valid)
                && report["checkpoint"] == frozen_checkpoint(report)?,
        )?;
        let preparations = report["preparations"]
            .as_object()
            .ok_or("owner_frozen_invalid")?;
        let sessions = report["sessions"]
            .as_object()
            .ok_or("owner_frozen_invalid")?;
        require(preparations.len() == 6 && sessions.len() == 6)?;
        for job in super::JOBS {
            let preparation = preparations.get(job).ok_or("owner_frozen_invalid")?;
            require(
                preparation["preparationSha256"]
                    .as_str()
                    .is_some_and(digest_valid),
            )?;
            require(if job == "assistance-crowd" {
                preparation["requestSha256"] == self.request_sha256
            } else {
                preparation["requestSha256"].is_null()
            })?;
            let matching: Vec<_> = sessions.values().filter(|s| s["job"] == job).collect();
            require(matching.len() == 1)?;
            let session = matching[0];
            require(
                session["stage"] == "preparation"
                    && session["closed"] == true
                    && session["finished"] == true
                    && session["attached"] == true
                    && session["sealed"] == true
                    && session["native_live"] == false
                    && session["fixture_live"] == false,
            )?;
        }
        require(
            report["counters"]["model"] == 0
                && report["counters"]["reserved"].as_u64().is_some()
                && report["counters"]["reserved"] == report["counters"]["arrived"]
                && report["counters"]["reserved"] == report["counters"]["completed"],
        )
    }
}

fn frozen_checkpoint(report: &Value) -> Result<String, String> {
    Ok(sha(&canonical(
        &json!({"runId":report["runId"],"membership":report["membership"],
        "bindings":report["bindings"],"preparations":report["preparations"],
        "services":report["sessions"],"counters":report["counters"]}),
        16 * MAX_RECORD,
    )?))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OwnerExecutionIntent {
    pub(crate) version: u8,
    pub(crate) proposal_id: String,
    pub(crate) proposal_sha256: String,
    pub(crate) execution_kind: ExecutionKind,
    pub(crate) nonce: String,
    pub(crate) issued_unix_seconds: u64,
    pub(crate) expires_unix_seconds: u64,
    pub(crate) hosted_estimate_risk_accepted: bool,
    pub(crate) signature: Vec<u8>,
}
impl OwnerExecutionIntent {
    fn signing_bytes(&self) -> Result<Vec<u8>, String> {
        let mut bytes = INTENT_DOMAIN.to_vec();
        bytes.extend(canonical(
            &json!({"version":self.version,"proposalId":self.proposal_id,
            "proposalSha256":self.proposal_sha256,"executionKind":self.execution_kind,
            "nonce":self.nonce,"issuedUnixSeconds":self.issued_unix_seconds,
            "expiresUnixSeconds":self.expires_unix_seconds,
            "hostedEstimateRiskAccepted":self.hosted_estimate_risk_accepted}),
            MAX_INTENT,
        )?);
        Ok(bytes)
    }
    pub(crate) fn sha256(&self) -> Result<String, String> {
        Ok(sha(&canonical(self, MAX_INTENT)?))
    }
}

/// Neither serializable, clonable, nor constructible by child requests.
pub(crate) struct VerifiedOwnerExecutionIntent {
    intent: OwnerExecutionIntent,
    verifier_sha256: String,
    directory: std::path::PathBuf,
}
/// A successfully fsynced no-replace claim; still not live-call authority.
pub(crate) struct ClaimedOwnerExecutionIntent {
    intent: OwnerExecutionIntent,
}

fn validate_intent(
    proposal: &ExecutionProposal,
    intent: &OwnerExecutionIntent,
    now: u64,
) -> Result<(), String> {
    require(
        intent.version == 1
            && intent.proposal_id == proposal.proposal_id
            && intent.proposal_sha256 == proposal.sha256()?
            && intent.execution_kind == proposal.execution_kind
            && uuid_valid(&intent.nonce)
            && intent.issued_unix_seconds > 0
            && intent.issued_unix_seconds <= now
            && intent.expires_unix_seconds > now
            && intent
                .expires_unix_seconds
                .checked_sub(intent.issued_unix_seconds)
                .is_some_and(|v| v <= MAX_LIFETIME)
            && intent.hosted_estimate_risk_accepted
                == (proposal.execution_kind == ExecutionKind::HostedSingleRequest),
    )
}

fn verify_at(
    proposal: &ExecutionProposal,
    intent: &OwnerExecutionIntent,
    public_key: &[u8; 32],
    now: u64,
) -> Result<(), String> {
    validate_intent(proposal, intent, now)?;
    require(sha(public_key) == proposal.verifier_sha256 && intent.signature.len() == 64)?;
    canonical(intent, MAX_INTENT)?;
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(&intent.signing_bytes()?, &intent.signature)
        .map_err(|_| "owner_intent_signature_invalid")?;
    Ok(())
}

#[cfg(unix)]
fn owner_process() -> Result<(), String> {
    if super::storage::is_scoped() {
        return Err("owner_intent_owner_process_required".into());
    }
    Ok(())
}

// The existing boundary's custody helpers are private to its module. Anchor
// these fixed-name reads/claims to an owner directory fd rather than widening
// their visibility or adding a generic public file/signing API.
#[cfg(unix)]
struct OwnerDirectory(std::fs::File);
#[cfg(unix)]
impl OwnerDirectory {
    fn open(path: &Path) -> Result<Self, String> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        owner_process()?;
        require(path.is_absolute() && path.canonicalize().ok().as_deref() == Some(path))?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| "owner_directory_unavailable")?;
        let m = file.metadata().map_err(|_| "owner_directory_unavailable")?;
        require(m.is_dir() && m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0)?;
        Ok(Self(file))
    }
    fn read(&self, name: &str, limit: u64) -> Result<Vec<u8>, String> {
        use std::io::Read;
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::MetadataExt;
        owner_process()?;
        let name = std::ffi::CString::new(name).map_err(|_| "owner_file_invalid")?;
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        require(fd >= 0)?;
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        let m = file.metadata().map_err(|_| "owner_file_unavailable")?;
        require(
            m.is_file()
                && m.uid() == unsafe { libc::geteuid() }
                && m.mode() & 0o077 == 0
                && m.nlink() == 1
                && m.len() <= limit,
        )?;
        let mut bytes = Vec::new();
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "owner_file_unavailable")?;
        require(bytes.len() as u64 <= limit)?;
        Ok(bytes)
    }
    fn verifier(&self, path: &Path, pin: &str) -> Result<[u8; 32], String> {
        owner_process()?;
        policy::boundary::ApprovalVerifier::read(&path.join("approval-verifier.ed25519"), pin)?;
        let bytes = self.read("approval-verifier.ed25519", 32)?;
        require(bytes.len() == 32 && sha(&bytes) == pin)?;
        bytes
            .try_into()
            .map_err(|_| "owner_verifier_invalid".into())
    }
}

/// Explicit owner-side intent issuance only. It does not publish approval,
/// consume the intent, start a clock, open credentials, or enable any transport.
#[cfg(unix)]
pub(crate) fn sign_owner_intent(
    directory: &Path,
    proposal: &ExecutionProposal,
    nonce: &str,
    issued: u64,
    expires: u64,
    hosted_estimate_risk_accepted: bool,
) -> Result<OwnerExecutionIntent, String> {
    owner_process()?;
    let mut intent = OwnerExecutionIntent {
        version: 1,
        proposal_id: proposal.proposal_id.clone(),
        proposal_sha256: proposal.sha256()?,
        execution_kind: proposal.execution_kind,
        nonce: nonce.into(),
        issued_unix_seconds: issued,
        expires_unix_seconds: expires,
        hosted_estimate_risk_accepted,
        signature: Vec::new(),
    };
    validate_intent(proposal, &intent, policy::now()?)?;
    require(current_executable_sha256()? == proposal.executable_sha256)?;
    let owner = OwnerDirectory::open(directory)?;
    let public = owner.verifier(directory, &proposal.verifier_sha256)?;
    let private = owner.read("approval-signing-key.pk8", 4096)?;
    let signer =
        Ed25519KeyPair::from_pkcs8_maybe_unchecked(&private).map_err(|_| "owner_signer_invalid")?;
    require(signer.public_key().as_ref() == public)?;
    intent.signature = signer.sign(&intent.signing_bytes()?).as_ref().to_vec();
    verify_at(proposal, &intent, &public, policy::now()?)?;
    Ok(intent)
}

#[cfg(unix)]
pub(crate) fn verify_owner_intent(
    directory: &Path,
    proposal: &ExecutionProposal,
    intent: &OwnerExecutionIntent,
    expected_kind: ExecutionKind,
    separately_pinned_verifier_sha256: &str,
) -> Result<VerifiedOwnerExecutionIntent, String> {
    owner_process()?;
    require(
        proposal.execution_kind == expected_kind
            && proposal.verifier_sha256 == separately_pinned_verifier_sha256,
    )?;
    validate_intent(proposal, intent, policy::now()?)?;
    require(current_executable_sha256()? == proposal.executable_sha256)?;
    let owner = OwnerDirectory::open(directory)?;
    verify_at(
        proposal,
        intent,
        &owner.verifier(directory, separately_pinned_verifier_sha256)?,
        policy::now()?,
    )?;
    Ok(VerifiedOwnerExecutionIntent {
        intent: intent.clone(),
        verifier_sha256: separately_pinned_verifier_sha256.into(),
        directory: directory.into(),
    })
}

impl VerifiedOwnerExecutionIntent {
    /// Consumes this capability even on error. A partially written file burns
    /// the nonce too; never delete it or retry an ambiguous claim.
    #[cfg(unix)]
    pub(crate) fn claim_once(
        self,
        directory: &Path,
    ) -> Result<ClaimedOwnerExecutionIntent, String> {
        use std::io::Write;
        use std::os::fd::{AsRawFd, FromRawFd};
        owner_process()?;
        require(
            directory == self.directory.as_path()
                && self.intent.expires_unix_seconds > policy::now()?,
        )?;
        let owner = OwnerDirectory::open(directory)?;
        owner.verifier(directory, &self.verifier_sha256)?;
        let bytes = canonical(
            &json!({"version":1,"intentSha256":self.intent.sha256()?,
            "proposalSha256":self.intent.proposal_sha256,"nonce":self.intent.nonce,
            "claimedUnixSeconds":policy::now()?,"liveAuthority":false}),
            MAX_INTENT,
        )?;
        let name = format!("intent-{}.claimed.json", self.intent.nonce);
        let c_name = std::ffi::CString::new(name.as_str()).map_err(|_| "owner_claim_invalid")?;
        let fd = unsafe {
            libc::openat(
                owner.0.as_raw_fd(),
                c_name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err("owner_intent_claimed_or_unavailable_no_retry".into());
        }
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        // Sync the new name even when data writing fails. Any failure after
        // create is ambiguous and leaves the no-replace tombstone intact.
        let written = file.write_all(&bytes).and_then(|_| file.sync_all());
        let synced = owner.0.sync_all();
        if written.is_err() || synced.is_err() {
            return Err("owner_intent_claim_unknown_burned_no_retry".into());
        }
        if owner.read(&name, MAX_INTENT as u64).ok().as_deref() != Some(bytes.as_slice()) {
            return Err("owner_intent_claim_unknown_burned_no_retry".into());
        }
        Ok(ClaimedOwnerExecutionIntent {
            intent: self.intent,
        })
    }
}

impl ClaimedOwnerExecutionIntent {
    /// Public bootstrap evidence only. The in-memory claimed capability, not
    /// this serializable binding, is required for the later checkpoint join.
    pub(crate) fn binding(&self) -> Result<Value, String> {
        Ok(json!({"version":1,"proposalId":self.intent.proposal_id,
            "proposalSha256":self.intent.proposal_sha256,"intentSha256":self.intent.sha256()?,
            "executionKind":self.intent.execution_kind,"nonce":self.intent.nonce,
            "expiresUnixSeconds":self.intent.expires_unix_seconds}))
    }

    pub(crate) fn validate_frozen_report(
        &self,
        proposal: &ExecutionProposal,
        report: &Value,
        plan: &policy::RehearsalPlan,
        context: &policy::Rehearsal,
    ) -> Result<(), String> {
        validate_intent(proposal, &self.intent, policy::now()?)?;
        require(report["bindings"]["ownerExecution"] == self.binding()?)?;
        proposal.validate_frozen_report(report, plan, context)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        root: std::path::PathBuf,
        proposal: ExecutionProposal,
        plan: policy::RehearsalPlan,
        context: policy::Rehearsal,
        membership: Value,
        bindings: Value,
    }
    impl Fixture {
        fn new(kind: ExecutionKind) -> Self {
            let root =
                std::env::temp_dir().join(format!("soundrobe-owner-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            let root = root.canonicalize().unwrap();
            let key = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
            let signer = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
            for (name, bytes) in [
                ("approval-signing-key.pk8", key.as_ref()),
                ("approval-verifier.ed25519", signer.public_key().as_ref()),
            ] {
                let path = root.join(name);
                std::fs::write(&path, bytes).unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            let plan: policy::RehearsalPlan = serde_json::from_value(json!({
                "nativePrepared":kind == ExecutionKind::HostedSingleRequest,
                "model":"~deepseek/deepseek-v4-flash-latest", "observedTarget":"deepseek/deepseek-v4-flash-0731",
                "route":"wafer/us", "provider":"Wafer", "input":{"artist":"Public Test Artist","album":"Public Test Album","tracks":["Public Track"]}
            })).unwrap();
            let pricing = policy::boundary::TextPricing {
                prompt_pico_usd: 1,
                completion_pico_usd: 2,
                catalog_sha256: sha(b"fake public catalog"),
                terms_sha256: sha(b"fake public terms"),
            };
            let context = plan
                .disabled_context(signer.public_key().as_ref().try_into().unwrap(), pricing)
                .unwrap();
            let membership: Value = serde_json::from_str(include_str!(
                "../../../test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json"
            ))
            .unwrap();
            let bindings = json!({"executable":current_executable_sha256().unwrap(),
                "modelRehearsalPlan":plan,"sourcesAndTools":[{"path":"public/fake-tool.rs","sha256":sha(b"public fake source")}]});
            let proposal = ExecutionProposal::construct(
                kind,
                &uuid::Uuid::new_v4().to_string(),
                &membership,
                &bindings,
                &plan,
                &context,
                &sha(signer.public_key().as_ref()),
            )
            .unwrap();
            Self {
                root,
                proposal,
                plan,
                context,
                membership,
                bindings,
            }
        }
        fn sign(&self) -> OwnerExecutionIntent {
            let now = policy::now().unwrap();
            sign_owner_intent(
                &self.root,
                &self.proposal,
                &uuid::Uuid::new_v4().to_string(),
                now,
                now + 1200,
                self.proposal.execution_kind == ExecutionKind::HostedSingleRequest,
            )
            .unwrap()
        }
        fn verify(
            &self,
            intent: &OwnerExecutionIntent,
        ) -> Result<VerifiedOwnerExecutionIntent, String> {
            verify_owner_intent(
                &self.root,
                &self.proposal,
                intent,
                self.proposal.execution_kind,
                &self.proposal.verifier_sha256,
            )
        }
        fn report(&self) -> Value {
            let mut bindings = self.bindings.clone();
            bindings["modelRehearsal"] = self.context.binding();
            let mut preparations = json!({});
            let mut sessions = json!({});
            for job in super::super::JOBS {
                preparations[job] = json!({"preparationSha256":sha(job.as_bytes()),
                    "requestSha256":if job == "assistance-crowd" { json!(self.proposal.request_sha256) } else { Value::Null }});
                sessions[job] = json!({"job":job,"stage":"preparation","closed":true,"finished":true,
                    "attached":true,"sealed":true,"native_live":false,"fixture_live":false});
            }
            let mut report = json!({"runId":uuid::Uuid::new_v4().to_string(),"state":"frozen", "ownerLive":true,
                "consumptions":0,"credentialLookups":0,"liveAuthority":false,"membership":self.membership,
                "bindings":bindings,"preparations":preparations,"sessions":sessions,"counters":{"model":0,"reserved":0,"arrived":0,"completed":0}});
            report["checkpoint"] = json!(frozen_checkpoint(&report).unwrap());
            report
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn owner_intent_is_explicit_kind_bound_and_claimed_once() {
        for kind in [
            ExecutionKind::OfflineRehearsal,
            ExecutionKind::HostedSingleRequest,
        ] {
            let f = Fixture::new(kind);
            let intent = f.sign();
            let verified = f.verify(&intent).unwrap();
            let claimed = verified.claim_once(&f.root).unwrap();
            let mut report = f.report();
            report["bindings"]["ownerExecution"] = claimed.binding().unwrap();
            report["checkpoint"] = json!(frozen_checkpoint(&report).unwrap());
            claimed
                .validate_frozen_report(&f.proposal, &report, &f.plan, &f.context)
                .unwrap();
            for field in [
                "proposalId",
                "proposalSha256",
                "intentSha256",
                "executionKind",
                "nonce",
                "expiresUnixSeconds",
            ] {
                let mut changed = report.clone();
                changed["bindings"]["ownerExecution"][field] = Value::Null;
                changed["checkpoint"] = json!(frozen_checkpoint(&changed).unwrap());
                assert!(
                    claimed
                        .validate_frozen_report(&f.proposal, &changed, &f.plan, &f.context)
                        .is_err(),
                    "{field}"
                );
            }
            assert!(claimed
                .validate_frozen_report(&f.proposal, &f.report(), &f.plan, &f.context)
                .is_err());
            assert!(f.verify(&intent).unwrap().claim_once(&f.root).is_err());
            let bytes = std::fs::read(f.root.join(format!("intent-{}.claimed.json", intent.nonce)))
                .unwrap();
            assert!(bytes.len() <= 8192);
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap()["intentSha256"],
                intent.sha256().unwrap()
            );
        }
    }

    #[test]
    fn owner_intent_rejects_tampering_wrong_pins_kinds_and_unbounded_time() {
        let f = Fixture::new(ExecutionKind::HostedSingleRequest);
        let intent = f.sign();
        for field in [
            "version",
            "proposalSha256",
            "proposalId",
            "executionKind",
            "nonce",
            "issuedUnixSeconds",
            "expiresUnixSeconds",
            "hostedEstimateRiskAccepted",
            "signature",
        ] {
            let mut value = serde_json::to_value(&intent).unwrap();
            value[field] = Value::Null;
            assert!(
                serde_json::from_value::<OwnerExecutionIntent>(value)
                    .map_or(true, |i| f.verify(&i).is_err()),
                "{field}"
            );
        }
        let now = policy::now().unwrap();
        for (issued, expires, risk) in [
            (now, now, true),
            (now + 60, now + 120, true),
            (now, now + 1801, true),
            (now, now + 60, false),
            (0, u64::MAX, true),
        ] {
            assert!(sign_owner_intent(
                &f.root,
                &f.proposal,
                &uuid::Uuid::new_v4().to_string(),
                issued,
                expires,
                risk
            )
            .is_err());
        }
        assert!(verify_owner_intent(
            &f.root,
            &f.proposal,
            &intent,
            ExecutionKind::OfflineRehearsal,
            &f.proposal.verifier_sha256
        )
        .is_err());
        assert!(verify_owner_intent(
            &f.root,
            &f.proposal,
            &intent,
            f.proposal.execution_kind,
            &sha(b"wrong pin")
        )
        .is_err());
        assert!(verify_at(&f.proposal, &intent, &[0; 32], intent.expires_unix_seconds).is_err());
        let mut value = serde_json::to_value(&intent).unwrap();
        value["extra"] = json!(true);
        assert!(serde_json::from_value::<OwnerExecutionIntent>(value).is_err());
    }

    #[test]
    fn frozen_report_rejects_membership_request_source_and_checkpoint_drift() {
        let f = Fixture::new(ExecutionKind::HostedSingleRequest);
        for pointer in [
            "/membership/requestedFieldSlots",
            "/bindings/executable",
            "/bindings/modelRehearsalPlan/model",
            "/bindings/sourcesAndTools/0/sha256",
            "/bindings/modelRehearsal/requestSha256",
            "/preparations/assistance-crowd/requestSha256",
            "/sessions/disabled-crowd/closed",
            "/consumptions",
            "/ownerLive",
            "/checkpoint",
        ] {
            let mut changed = f.report();
            *changed.pointer_mut(pointer).unwrap() = Value::Null;
            assert!(
                f.proposal
                    .validate_frozen_report(&changed, &f.plan, &f.context)
                    .is_err(),
                "{pointer}"
            );
        }
        let mut changed = f.report();
        changed["bindings"]["sourcesAndTools"][0]["sha256"] = json!(sha(b"changed source"));
        changed["checkpoint"] = json!(frozen_checkpoint(&changed).unwrap());
        assert!(f
            .proposal
            .validate_frozen_report(&changed, &f.plan, &f.context)
            .is_err());
    }

    #[test]
    fn proposal_rejects_fixture_keys_and_malformed_digests() {
        let f = Fixture::new(ExecutionKind::HostedSingleRequest);
        for seed in [42, 43] {
            let fixture = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
            assert!(ExecutionProposal::construct(
                f.proposal.execution_kind,
                &f.proposal.proposal_id,
                &f.membership,
                &f.bindings,
                &f.plan,
                &f.context,
                &sha(fixture.public_key().as_ref())
            )
            .is_err());
        }
        for digest in ["", "UPPERCASE", &"a".repeat(63)] {
            let mut changed = f.proposal.clone();
            changed.request_sha256 = digest.into();
            assert!(changed.sha256().is_err());
        }
    }

    #[test]
    fn claim_never_overwrites_an_existing_partial_record() {
        let f = Fixture::new(ExecutionKind::HostedSingleRequest);
        let intent = f.sign();
        let target = f.root.join(format!("intent-{}.claimed.json", intent.nonce));
        std::fs::write(&target, b"partial unknown claim").unwrap();
        assert!(f.verify(&intent).unwrap().claim_once(&f.root).is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"partial unknown claim");
    }

    #[test]
    fn owner_intent_signature_is_domain_separated_and_canonical() {
        let f = Fixture::new(ExecutionKind::HostedSingleRequest);
        let mut intent = f.sign();
        let original = intent.signature.clone();
        intent.signature[0] ^= 1;
        assert!(f.verify(&intent).is_err());
        intent.signature = original;
        let bytes = intent.signing_bytes().unwrap();
        let claims: Value = serde_json::from_slice(&bytes[INTENT_DOMAIN.len()..]).unwrap();
        assert_eq!(
            &bytes[INTENT_DOMAIN.len()..],
            canonical(&claims, MAX_INTENT).unwrap()
        );
        let private = std::fs::read(f.root.join("approval-signing-key.pk8")).unwrap();
        let signer = Ed25519KeyPair::from_pkcs8(&private).unwrap();
        intent.signature = signer.sign(&bytes[INTENT_DOMAIN.len()..]).as_ref().to_vec();
        assert!(f.verify(&intent).is_err());
        intent.signature = signer.sign(&bytes).as_ref().to_vec();
        assert!(f.verify(&intent).is_ok());
        intent.nonce = uuid::Uuid::new_v4().to_string();
        assert!(f.verify(&intent).is_err());
    }

    #[test]
    fn owner_intent_custody_rejects_symlinks_and_public_private_keys() {
        let f = Fixture::new(ExecutionKind::HostedSingleRequest);
        let key = f.root.join("approval-signing-key.pk8");
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        let now = policy::now().unwrap();
        assert!(sign_owner_intent(
            &f.root,
            &f.proposal,
            &uuid::Uuid::new_v4().to_string(),
            now,
            now + 60,
            true
        )
        .is_err());
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        let alias = f.root.join("alias");
        std::os::unix::fs::symlink(&f.root, &alias).unwrap();
        assert!(sign_owner_intent(
            &alias,
            &f.proposal,
            &uuid::Uuid::new_v4().to_string(),
            now,
            now + 60,
            true
        )
        .is_err());
        let intent = f.sign();
        assert!(f.verify(&intent).unwrap().claim_once(&alias).is_err());
    }

    #[test]
    fn hosted_proposal_rejects_estimate_at_the_allowance() {
        let f = Fixture::new(ExecutionKind::HostedSingleRequest);
        let public = std::fs::read(f.root.join("approval-verifier.ed25519")).unwrap();
        let context = f
            .plan
            .disabled_context(
                public.try_into().unwrap(),
                policy::boundary::TextPricing {
                    prompt_pico_usd: 0,
                    completion_pico_usd: 4_882_812_500,
                    catalog_sha256: sha(b"fake catalog"),
                    terms_sha256: sha(b"fake terms"),
                },
            )
            .unwrap();
        assert!(ExecutionProposal::construct(
            f.proposal.execution_kind,
            &f.proposal.proposal_id,
            &f.membership,
            &f.bindings,
            &f.plan,
            &context,
            &f.proposal.verifier_sha256
        )
        .is_err());
    }

    #[test]
    fn proposal_normalizes_only_the_exact_runtime_supervision_slot() {
        let mut f = Fixture::new(ExecutionKind::HostedSingleRequest);
        let relative = format!(
            ".amp/in/artifacts/public-owner-test-{}/native-supervision.json",
            uuid::Uuid::new_v4()
        );
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        f.bindings["disposableRunRoot"] = json!(workspace.join(&relative).parent().unwrap());
        f.bindings["sourcesAndTools"]
            .as_array_mut()
            .unwrap()
            .push(runtime_marker(&relative));
        f.proposal = ExecutionProposal::construct(
            f.proposal.execution_kind,
            &f.proposal.proposal_id,
            &f.membership,
            &f.bindings,
            &f.plan,
            &f.context,
            &f.proposal.verifier_sha256,
        )
        .unwrap();
        let generation = uuid::Uuid::new_v4().to_string();
        let mib = 1024 * 1024u64;
        let config = json!({"generation":generation,"start":12345,"preparationMs":300000,"executionMs":600000,
            "receiverPid":101,"receiverGroup":102,"control":"","storage":{
                "generation":generation,"logicalLimit":512*mib,"applicationWriteLimit":1024*mib,
                "terminalGrowthReserved":2*mib,"terminalWritesReserved":8*mib,"selfGrowthConsumed":2*mib,
                "selfWritesConsumed":4*mib,"maximumOwners":255,"failedBookkeepingReserved":128*1024,
                "launcherTerminalPartition":64*1024,"coverage":"evaluation_files_sqlite_helpers_prepaid_not_physical_io"}});
        let mut report = f.report();
        report["bindings"]["sourcesAndTools"][1] =
            json!({"path":relative,"sha256":sha(&canonical(&config,MAX_RECORD).unwrap())});
        report["bindings"]["supervision"] = config;
        report["bindings"]["supervision"]["executionGroup"] = json!(103);
        report["checkpoint"] = json!(frozen_checkpoint(&report).unwrap());
        f.proposal
            .validate_frozen_report(&report, &f.plan, &f.context)
            .unwrap();
        for (pointer, value) in [
            (
                "/bindings/sourcesAndTools/1/path",
                json!("public/substituted/native-supervision.json"),
            ),
            (
                "/bindings/sourcesAndTools/1/sha256",
                json!(sha(b"substituted runtime")),
            ),
            (
                "/bindings/sourcesAndTools/0/sha256",
                json!(sha(b"changed static input")),
            ),
            ("/bindings/supervision/start", json!(12346)),
            (
                "/bindings/supervision/storage/logicalLimit",
                json!(1024 * mib),
            ),
            ("/bindings/supervision/executionGroup", json!(102)),
            ("/bindings/supervision/unknown", json!(true)),
        ] {
            let mut changed = report.clone();
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            changed.pointer_mut(parent).unwrap()[key] = value;
            changed["checkpoint"] = json!(frozen_checkpoint(&changed).unwrap());
            assert!(
                f.proposal
                    .validate_frozen_report(&changed, &f.plan, &f.context)
                    .is_err(),
                "{pointer}"
            );
        }
        let mut missing = report.clone();
        missing["bindings"]["sourcesAndTools"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(f
            .proposal
            .validate_frozen_report(&missing, &f.plan, &f.context)
            .is_err());
        let mut duplicate = report.clone();
        duplicate["bindings"]["sourcesAndTools"]
            .as_array_mut()
            .unwrap()
            .push(report["bindings"]["sourcesAndTools"][1].clone());
        assert!(f
            .proposal
            .validate_frozen_report(&duplicate, &f.plan, &f.context)
            .is_err());
        assert!(f
            .proposal
            .validate_frozen_report(&f.report(), &f.plan, &f.context)
            .is_err());
    }

    #[test]
    fn scoped_evaluator_cannot_read_signer_or_claim_owner_intent() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "infra::mock_run_authority::owner::tests::owner_scope_guard_child",
                "--nocapture",
            ])
            .env("SOUNDROBE_OWNER_SCOPE_TEST_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn owner_scope_guard_child() {
        if std::env::var("SOUNDROBE_OWNER_SCOPE_TEST_CHILD").as_deref() != Ok("1") {
            return;
        }
        let f = Fixture::new(ExecutionKind::HostedSingleRequest);
        let intent = f.sign();
        let verified = f.verify(&intent).unwrap();
        let account = f.root.join("account");
        std::fs::create_dir(&account).unwrap();
        super::super::storage::install_receiver(&account, uuid::Uuid::new_v4()).unwrap();
        let absent = f.root.join("must-not-be-opened");
        let expected = "owner_intent_owner_process_required";
        let now = policy::now().unwrap();
        assert_eq!(
            sign_owner_intent(&absent, &f.proposal, &intent.nonce, now, now + 60, true)
                .err()
                .as_deref(),
            Some(expected)
        );
        assert_eq!(
            verify_owner_intent(
                &absent,
                &f.proposal,
                &intent,
                f.proposal.execution_kind,
                &f.proposal.verifier_sha256
            )
            .err()
            .as_deref(),
            Some(expected)
        );
        assert_eq!(
            verified.claim_once(&absent).err().as_deref(),
            Some(expected)
        );
        assert!(!absent.exists());
        assert!(!f
            .root
            .join(format!("intent-{}.claimed.json", intent.nonce))
            .exists());
    }
}
