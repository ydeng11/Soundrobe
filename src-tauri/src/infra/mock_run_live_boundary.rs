//! Disabled hosted-request preparation; no activation or signer provisioning.
use super::*;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

pub(crate) const DISABLED_POLICY: &str = "disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair";
pub(crate) const ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HttpTransport {
    HostedHttps,
    OfflineLocalHttp,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HttpUsage {
    pub(crate) prompt_tokens: u64,
    pub(crate) completion_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) reasoning_tokens: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HttpProjection {
    pub(crate) model: String,
    pub(crate) provider: String,
    pub(crate) genre: String,
    pub(crate) confidence: f64,
    pub(crate) usage: HttpUsage,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HttpValidation {
    Accepted,
    Withheld,
    Rejected,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HttpObservation {
    pub(crate) version: u8,
    pub(crate) transport: HttpTransport,
    pub(crate) request_sha256: String,
    pub(crate) validation: HttpValidation,
    pub(crate) projection: Option<HttpProjection>,
}
impl HttpObservation {
    pub(crate) fn validate(&self, context: &Rehearsal, status: u16) -> Result<(), String> {
        require(self.version == 1 && self.request_sha256 == sha(&context.body))?;
        match (&self.projection, self.validation) {
            (None, HttpValidation::Rejected) => Ok(()),
            (Some(p), validation) => {
                require(
                    status == 200
                        && p.model == context.resolved_model
                        && p.provider == context.provider
                        && p.genre.len() <= 256
                        && !p.genre.chars().any(char::is_control)
                        && p.confidence.is_finite()
                        && (0.0..=1.0).contains(&p.confidence)
                        && p.usage.prompt_tokens <= 8192
                        && p.usage.completion_tokens <= 1024
                        && p.usage.total_tokens
                            == p.usage.prompt_tokens + p.usage.completion_tokens
                        && p.usage.reasoning_tokens == 0,
                )?;
                require(if validation == HttpValidation::Accepted {
                    !p.genre.is_empty() && p.confidence >= 0.6
                } else {
                    validation == HttpValidation::Withheld && p.genre.is_empty()
                })
            }
            _ => Err("mock_http_observation_invalid".into()),
        }
    }
}

/// Public, native-supplied evidence, not provider-side price enforcement.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TextPricing {
    pub(crate) prompt_pico_usd: u64,
    pub(crate) completion_pico_usd: u64,
    pub(crate) catalog_sha256: String,
    pub(crate) terms_sha256: String,
}
impl RehearsalPlan {
    pub(crate) fn disabled_context(
        &self,
        public_key: [u8; 32],
        pricing: TextPricing,
    ) -> Result<Rehearsal, String> {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let fixture = Ed25519KeyPair::from_seed_unchecked(&[42; 32]).unwrap();
        require(public_key != fixture.public_key().as_ref())?;
        let estimate = u128::from(pricing.prompt_pico_usd) * 8192
            + u128::from(pricing.completion_pico_usd) * 1024;
        require(
            estimate <= 5_000_000_000_000
                && [&pricing.catalog_sha256, &pricing.terms_sha256]
                    .iter()
                    .all(|s| super::super::digest_valid(s)),
        )?;
        let mut context = Rehearsal::new(
            &self.input,
            &self.model,
            &self.observed_target,
            &self.route,
            &self.provider,
            public_key,
        )?;
        context.boundary = Some(
            json!({"endpoint":ENDPOINT,"settingsSha256":settings_sha(&self.model),
            "pricingEvidence":pricing,"textEstimatePicoUsd":estimate as u64,"allowanceUsdCents":500,
            "inputTokenIllustration":8192,"outputTokens":1024,"hardBillingCap":false,
            "billingRiskAccepted":false,"liveDispatchAvailable":false,"readyForExecution":false,
            "retention":{"remote":"documented_provider_defaults_for_public_metadata","local":"until_result_review"}}),
        );
        Ok(context)
    }
}
fn settings_sha(model: &str) -> String {
    // Field order matches the maintained file-only preflight, not a private-file hash.
    #[derive(Serialize)]
    struct Settings<'a> {
        model: &'a str,
        endpoint: &'a str,
        provider: &'a str,
        source: &'a str,
    }
    sha(&serde_json::to_vec(&Settings {
        model,
        endpoint: "https://openrouter.ai/api/v1",
        provider: "openrouter",
        source: "env_local_only",
    })
    .unwrap())
}

// Shared bounded custody checks for native trust/configuration/approval files.
fn owner_bytes(path: &std::path::Path, limit: u64) -> Result<Vec<u8>, String> {
    require(path.is_absolute())?;
    require(path.parent().and_then(|p| p.canonicalize().ok()).as_deref() == path.parent())?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "approval_file_missing"
            } else {
                "approval_file_unavailable"
            }
        })?;
    let metadata = file.metadata().map_err(|_| "approval_file_unavailable")?;
    require(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1
            && metadata.len() <= limit,
    )?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "approval_file_unavailable")?;
    require(bytes.len() as u64 <= limit)?;
    Ok(bytes)
}
fn owner_directory(path: &std::path::Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "approval_directory_unavailable")?;
    require(
        path.is_absolute()
            && path.canonicalize().ok().as_deref() == Some(path)
            && metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
    )
}

/// Native-owned bootstrap capability; never deserialized from child IPC.
/// No signer is loaded, and credential bytes remain lazy behind closed dispatch.
pub(crate) struct PreparedBootstrap {
    pub(crate) context: Rehearsal,
    #[cfg_attr(
        not(feature = "server"),
        expect(dead_code, reason = "Consumed by the server-only broker")
    )]
    pub(crate) credential: CredentialFile,
    #[cfg_attr(
        not(feature = "server"),
        expect(dead_code, reason = "Consumed by the server-only broker")
    )]
    pub(crate) approval_file: std::path::PathBuf,
}
impl PreparedBootstrap {
    #[cfg(all(feature = "server", target_os = "macos"))]
    pub(crate) fn owner_directory() -> Result<std::path::PathBuf, String> {
        use std::os::unix::ffi::OsStringExt;
        // Account database, not HOME or a launcher-provided trust-directory override.
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0u8; 16384];
        let code = unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        require(code == 0 && !result.is_null())?;
        let entry = unsafe { entry.assume_init() };
        require(!entry.pw_dir.is_null())?;
        let home = unsafe { std::ffi::CStr::from_ptr(entry.pw_dir) }.to_bytes();
        Ok(
            std::path::PathBuf::from(std::ffi::OsString::from_vec(home.to_vec()))
                .join(".soundrobe/evaluation-approval"),
        )
    }
    pub(crate) fn read(directory: &std::path::Path, plan: &RehearsalPlan) -> Result<Self, String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Config {
            version: u8,
            plan_sha256: String,
            verifier_sha256: String,
            credential_path: std::path::PathBuf,
            pricing: TextPricing,
        }
        require(plan.native_prepared)?;
        owner_directory(directory)?;
        let config: Config = serde_json::from_slice(&owner_bytes(
            &directory.join("prepared-bootstrap.json"),
            16384,
        )?)
        .map_err(|_| "approval_bootstrap_invalid")?;
        require(
            config.version == 1 && config.plan_sha256 == sha(&serde_json::to_vec(plan).unwrap()),
        )?;
        let verifier = ApprovalVerifier::read(
            &directory.join("approval-verifier.ed25519"),
            &config.verifier_sha256,
        )?;
        let context = plan.prepare_hosted(&verifier, config.pricing)?;
        let credential = CredentialFile::new(config.credential_path, plan.model.clone())
            .map_err(str::to_owned)?;
        Ok(Self {
            context,
            credential,
            approval_file: directory.join("prepared-approval.json"),
        })
    }
}
impl Rehearsal {
    // Explicit owner-side call only; never reached by broker IPC or the launcher.
    #[cfg(feature = "server")]
    pub(in super::super) fn sign_owner_payload(
        &self,
        directory: &std::path::Path,
        payload: Value,
    ) -> Result<Approval, String> {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        require(
            self.disabled_hosted()
                && !self.offline_fixture
                && !self.offline_http
                && self.offline_response.is_none(),
        )?;
        owner_directory(directory)?;
        ApprovalVerifier::read(
            &directory.join("approval-verifier.ed25519"),
            &sha(&self.public_key),
        )?;
        let private = owner_bytes(&directory.join("approval-signing-key.pk8"), 4096)?;
        // OpenSSL generates PKCS8 v1 without a public component. Ring derives it;
        // compare it with the separately pinned verifier rather than trusting it.
        let signer = Ed25519KeyPair::from_pkcs8_maybe_unchecked(&private)
            .map_err(|_| "approval_signer_invalid")?;
        require(signer.public_key().as_ref() == self.public_key)?;
        let bytes = serde_json::to_vec(&payload).map_err(|_| "mock_approval_invalid")?;
        require(bytes.len() <= 4096)?;
        Ok(Approval {
            payload,
            signature: signer.sign(&bytes).as_ref().to_vec(),
        })
    }
}

/// No-replace publication is supported only on the verified macOS boundary.
/// A failure after rename is unknown publication, never permission to retry.
#[cfg(all(feature = "server", target_os = "macos"))]
pub(crate) fn publish_owner_approval(
    directory: &std::path::Path,
    approval: &Approval,
) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    if super::super::storage::is_scoped() {
        return Err("approval_publication_owner_process_required".into());
    }
    owner_directory(directory)?;
    let bytes = serde_json::to_vec(approval).map_err(|_| "approval_file_invalid")?;
    require(bytes.len() <= 8192)?;
    let target = directory.join("prepared-approval.json");
    let temporary = directory.join(format!(".prepared-approval-{}", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)
        .map_err(|_| "approval_publication_unavailable")?;
    let result = (|| {
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "approval_publication_unavailable")?;
        let from = std::ffi::CString::new(temporary.as_os_str().as_bytes())
            .map_err(|_| "approval_publication_unavailable")?;
        let to = std::ffi::CString::new(target.as_os_str().as_bytes())
            .map_err(|_| "approval_publication_unavailable")?;
        require(
            unsafe {
                libc::renameatx_np(
                    libc::AT_FDCWD,
                    from.as_ptr(),
                    libc::AT_FDCWD,
                    to.as_ptr(),
                    libc::RENAME_EXCL,
                )
            } == 0,
        )
    })();
    drop(file);
    if let Err(error) = result {
        std::fs::remove_file(&temporary).map_err(|_| "approval_publication_cleanup_unknown")?;
        return Err(error);
    }
    std::fs::File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| "approval_publication_unknown")?;
    let observed = owner_bytes(&target, 8192).map_err(|_| "approval_publication_unknown")?;
    if observed != bytes {
        return Err("approval_publication_unknown".into());
    }
    Ok(())
}
#[cfg(all(feature = "server", not(target_os = "macos")))]
pub(crate) fn publish_owner_approval(
    _directory: &std::path::Path,
    _approval: &Approval,
) -> Result<(), String> {
    Err("approval_publication_unsupported".into())
}

#[cfg(feature = "server")]
pub(crate) fn read_owner_approval(path: &std::path::Path) -> Result<Approval, String> {
    owner_directory(path.parent().ok_or("approval_file_unavailable")?)?;
    serde_json::from_slice(&owner_bytes(path, 8192)?).map_err(|_| "approval_file_invalid".into())
}

/// An independently pinned owner file, never a plan/IPC-supplied verifier.
/// Loading public bytes does not provision a signer or enable hosted dispatch.
pub(crate) struct ApprovalVerifier {
    public_key: [u8; 32],
}
impl ApprovalVerifier {
    pub(crate) fn read(path: &std::path::Path, expected_sha256: &str) -> Result<Self, String> {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        require(super::super::digest_valid(expected_sha256))?;
        let bytes = owner_bytes(path, 32).map_err(|_| "approval_verifier_unavailable")?;
        require(
            bytes.len() == 32 && sha(&bytes) == expected_sha256 && bytes.iter().any(|b| *b != 0),
        )?;
        require(bytes != TEST_PUBLIC_KEY)?;
        for seed in [42, 43] {
            let fixture = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
            require(bytes != fixture.public_key().as_ref())?;
        }
        Ok(Self {
            public_key: bytes.try_into().unwrap(),
        })
    }
}
impl RehearsalPlan {
    pub(crate) fn prepare_hosted(
        &self,
        verifier: &ApprovalVerifier,
        pricing: TextPricing,
    ) -> Result<Rehearsal, String> {
        require(
            self.offline_hosted.is_none() && !self.offline_http && self.offline_response.is_none(),
        )?;
        self.disabled_context(verifier.public_key, pricing)
    }
}

/// Native-only capability. No IPC/serde constructor, environment fallback or key Debug.
/// Owner-only Unix file access is not malicious-host isolation or memory zeroization.
pub(crate) struct CredentialFile {
    path: std::path::PathBuf,
    model: String,
}
impl CredentialFile {
    pub(crate) fn new(path: std::path::PathBuf, model: String) -> Result<Self, &'static str> {
        if !path.is_absolute() || path.file_name() != Some(std::ffi::OsStr::new(".env.local")) {
            return Err("model_credential_unavailable");
        }
        let parent = path
            .parent()
            .unwrap()
            .canonicalize()
            .map_err(|_| "model_credential_unavailable")?;
        Ok(Self {
            path: parent.join(".env.local"),
            model,
        })
    }
    pub(crate) fn settings_sha(&self) -> String {
        settings_sha(&self.model)
    }
    pub(crate) fn read(&self) -> Result<String, &'static str> {
        self.read_inner().ok_or("model_credential_unavailable")
    }
    fn read_inner(&self) -> Option<String> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&self.path)
            .ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
            || metadata.len() > 65536
        {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes).ok()?;
        if bytes.len() > 65536 {
            return None;
        }
        let text = std::str::from_utf8(&bytes).ok()?;
        let mut values = std::collections::BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
            let (name, value) = line.split_once('=')?;
            let name = name.trim();
            if !["LLM_API_KEY", "LLM_MODEL", "LLM_PROVIDER", "LLM_BASE_URL"].contains(&name) {
                continue;
            }
            let value = value.trim();
            let value =
                if let Some(quote) = value.chars().next().filter(|c| *c == '\'' || *c == '"') {
                    let remaining = &value[1..];
                    let end = remaining.find(quote)?;
                    let trailing = remaining[end + 1..].trim();
                    if !trailing.is_empty() && !trailing.starts_with('#') {
                        return None;
                    }
                    // This strict subset deliberately rejects escapes/multiline values.
                    // Never shell-source or interpolate the file.
                    let value = &remaining[..end];
                    if value.contains('\\') {
                        return None;
                    }
                    value
                } else {
                    let value = value.split('#').next()?.trim();
                    if value.contains(['\'', '"', '`', '\\']) {
                        return None;
                    }
                    value
                };
            if value.chars().any(char::is_control) || values.insert(name, value).is_some() {
                return None;
            }
        }
        if values.get("LLM_MODEL") != Some(&self.model.as_str())
            || !matches!(
                values.get("LLM_PROVIDER"),
                None | Some(&"") | Some(&"openrouter")
            )
            || !matches!(
                values.get("LLM_BASE_URL"),
                None | Some(&"") | Some(&"https://openrouter.ai/api/v1")
            )
        {
            return None;
        }
        let key = *values.get("LLM_API_KEY")?;
        if key.is_empty()
            || key.len() > 4096
            || key.starts_with("****")
            || !key.bytes().all(|b| (33..=126).contains(&b))
        {
            return None;
        }
        Some(key.into())
    }
}

#[test]
fn prepared_bootstrap_uses_owner_files_without_reading_credentials_or_fixture_trust() {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::os::unix::fs::PermissionsExt;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.amp/in/approval-fixtures")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let key = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let signer = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
    let verifier = root.join("approval-verifier.ed25519");
    std::fs::write(&verifier, signer.public_key().as_ref()).unwrap();
    std::fs::set_permissions(&verifier, std::fs::Permissions::from_mode(0o600)).unwrap();
    let raw = json!({"nativePrepared":true,"model":"~deepseek/deepseek-v4-flash-latest",
        "observedTarget":"deepseek/deepseek-v4-flash-0731","route":"wafer/us","provider":"Wafer",
        "input":{"artist":"Asymmetric Artist","album":"Different Album","tracks":["One","Other"]}});
    let plan: RehearsalPlan = serde_json::from_value(raw.clone()).unwrap();
    assert!(
        plan.context().is_err(),
        "native plan must not fall back to seed42"
    );
    let config = json!({"version":1,"planSha256":sha(&serde_json::to_vec(&plan).unwrap()),
        "verifierSha256":sha(signer.public_key().as_ref()),"credentialPath":root.join("missing/.env.local"),
        "pricing":{"promptPicoUsd":1,"completionPicoUsd":2,"catalogSha256":sha(b"catalog"),"termsSha256":sha(b"terms")}});
    let file = root.join("prepared-bootstrap.json");
    let write = |value: &Value| {
        std::fs::write(&file, serde_json::to_vec(value).unwrap()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    write(&config);
    assert!(
        PreparedBootstrap::read(&root, &plan).is_err(),
        "missing credential parent"
    );
    std::fs::create_dir(root.join("missing")).unwrap();
    let prepared = PreparedBootstrap::read(&root, &plan).unwrap();
    assert_eq!(
        prepared.context.binding()["verifierSha256"],
        config["verifierSha256"]
    );
    assert_eq!(
        prepared.context.binding()["boundary"]["liveDispatchAvailable"],
        false
    );
    assert_eq!(
        prepared.context.binding()["boundary"]["allowanceUsdCents"],
        500
    );
    assert!(prepared.context.binding().get("offlineFixture").is_none());
    assert!(
        !root.join("missing/.env.local").exists(),
        "bootstrap must not need credential bytes"
    );
    for field in [
        "planSha256",
        "verifierSha256",
        "version",
        "liveDispatchAvailable",
        "pricing",
    ] {
        let mut bad = config.clone();
        bad[field] = match field {
            "version" => json!(2),
            "pricing" => {
                json!({"promptPicoUsd":u64::MAX,"completionPicoUsd":1,"catalogSha256":sha(b"catalog"),"termsSha256":sha(b"terms")})
            }
            "liveDispatchAvailable" => json!(true),
            _ => json!(sha(b"wrong")),
        };
        write(&bad);
        assert!(PreparedBootstrap::read(&root, &plan).is_err(), "{field}");
    }
    write(&config);
    for mode in [0o644, 0o620] {
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(PreparedBootstrap::read(&root, &plan).is_err());
    }
    write(&config);
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o750)).unwrap();
    assert!(PreparedBootstrap::read(&root, &plan).is_err());
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let alias = root.join("alias.json");
    std::fs::hard_link(&file, &alias).unwrap();
    assert!(PreparedBootstrap::read(&root, &plan).is_err());
    std::fs::remove_file(&alias).unwrap();
    std::fs::rename(&file, &alias).unwrap();
    std::os::unix::fs::symlink(&alias, &file).unwrap();
    assert!(PreparedBootstrap::read(&root, &plan).is_err());
    std::fs::remove_file(&file).unwrap();
    assert!(PreparedBootstrap::read(&root, &plan).is_err());
    write(&config);
    for marker in [json!(false), json!(1), Value::Null] {
        let mut bad = raw.clone();
        bad["nativePrepared"] = marker;
        assert!(serde_json::from_value::<RehearsalPlan>(bad)
            .map_or(true, |p| PreparedBootstrap::read(&root, &p).is_err()));
    }
    let mut bad = raw;
    bad["offlineHosted"] = config["pricing"].clone();
    let bad: RehearsalPlan = serde_json::from_value(bad).unwrap();
    let mut matching = config.clone();
    matching["planSha256"] = json!(sha(&serde_json::to_vec(&bad).unwrap()));
    write(&matching);
    assert!(PreparedBootstrap::read(&root, &bad).is_err());
    std::fs::write(&file, vec![b' '; 16385]).unwrap();
    assert!(PreparedBootstrap::read(&root, &plan).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_approval_verifier_is_owner_pinned_and_not_public_fixture_trust() {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::os::unix::fs::PermissionsExt;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.amp/in/approval-fixtures")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    let path = root
        .canonicalize()
        .unwrap()
        .join("approval-verifier.ed25519");
    // Disposable random signer exists only in this control, never provisioning.
    let key = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let signer = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
    let public = signer.public_key().as_ref();
    std::fs::write(&path, public).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let verifier = ApprovalVerifier::read(&path, &sha(public)).unwrap();
    let plan: RehearsalPlan =
        serde_json::from_value(json!({"model":"~deepseek/deepseek-v4-flash-latest",
        "observedTarget":"deepseek/deepseek-v4-flash-0731","route":"wafer/us","provider":"Wafer",
        "input":{"artist":"Asymmetric Artist","album":"Different Album","tracks":["One","Other"]}}))
        .unwrap();
    let pricing = TextPricing {
        prompt_pico_usd: 1,
        completion_pico_usd: 2,
        catalog_sha256: sha(b"public catalog"),
        terms_sha256: sha(b"public terms"),
    };
    let context = plan.prepare_hosted(&verifier, pricing.clone()).unwrap();
    assert_eq!(context.binding()["verifierSha256"], sha(public));
    assert!(context.binding().get("offlineFixture").is_none());
    assert_eq!(
        context.binding()["boundary"]["liveDispatchAvailable"],
        false
    );
    let request_sha = sha(&context.body);
    let (journal, owner) = super::super::Journal::create_rehearsal(
        &root.join("registry.sqlite"),
        serde_json::from_str(include_str!(
            "../../../test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json"
        ))
        .unwrap(),
        json!({"executable":sha(b"test executable")}),
        context,
    )
    .unwrap();
    journal
        .command(
            &owner,
            super::super::Command::Lease {
                role: "owner".into(),
            },
        )
        .unwrap();
    super::super::tests::freeze_request(&journal, &owner, Some(request_sha));
    let report = journal.report().unwrap();
    let scope = json!({"runId":report["runId"],"checkpoint":report["checkpoint"],"bindings":report["bindings"],
        "membership":report["membership"],"preparations":report["preparations"],"policy":report["testPolicy"]});
    let payload = json!({"version":1,"runId":report["runId"],"checkpoint":report["checkpoint"],
        "scopeSha256":sha(&serde_json::to_vec(&scope).unwrap()),"nonce":uuid::Uuid::new_v4().to_string(),
        "expiresUnixSeconds":super::now().unwrap()+1200});
    let approval = || {
        serde_json::from_value(json!({"signature":signer.sign(&serde_json::to_vec(&payload).unwrap()).as_ref(),"payload":payload})).unwrap()
    };
    journal
        .command(
            &owner,
            super::super::Command::Approve {
                approval: approval(),
            },
        )
        .unwrap();
    assert!(journal
        .command(
            &owner,
            super::super::Command::Approve {
                approval: approval()
            }
        )
        .is_err());
    assert_eq!(journal.report().unwrap()["liveAuthority"], false);
    assert!(ApprovalVerifier::read(&path, &sha(b"wrong pin")).is_err());
    for mode in [0o644, 0o620] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(ApprovalVerifier::read(&path, &sha(public)).is_err());
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let alias = root.join("alias");
    std::fs::hard_link(&path, &alias).unwrap();
    assert!(ApprovalVerifier::read(&path, &sha(public)).is_err());
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert!(ApprovalVerifier::read(&alias, &sha(public)).is_err());
    std::fs::remove_file(&alias).unwrap();
    for raw in [
        vec![],
        vec![0; 32],
        vec![1; 33],
        TEST_PUBLIC_KEY.to_vec(),
        Ed25519KeyPair::from_seed_unchecked(&[42; 32])
            .unwrap()
            .public_key()
            .as_ref()
            .to_vec(),
        Ed25519KeyPair::from_seed_unchecked(&[43; 32])
            .unwrap()
            .public_key()
            .as_ref()
            .to_vec(),
    ] {
        std::fs::write(&path, &raw).unwrap();
        assert!(ApprovalVerifier::read(&path, &sha(&raw)).is_err());
    }
    let mut fixture = serde_json::to_value(&plan).unwrap();
    fixture["offlineHosted"] = serde_json::to_value(&pricing).unwrap();
    assert!(serde_json::from_value::<RehearsalPlan>(fixture)
        .unwrap()
        .prepare_hosted(&verifier, pricing)
        .is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn disabled_boundary_keeps_external_request_exact_but_rejects_fixture_trust() {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let plan: RehearsalPlan = serde_json::from_value(json!({"model":"~deepseek/deepseek-v4-flash-latest",
        "observedTarget":"deepseek/deepseek-v4-flash-0731","route":"wafer/us","provider":"Wafer",
        "input":{"artist":"Asymmetric Artist","album":"Different Album","tracks":["Track One","Other Two"]}})).unwrap();
    let evidence = TextPricing {
        prompt_pico_usd: 1000,
        completion_pico_usd: 2000,
        catalog_sha256: sha(b"public catalog"),
        terms_sha256: sha(b"public terms"),
    };
    let signer = Ed25519KeyPair::from_seed_unchecked(&[43; 32]).unwrap();
    let context = plan
        .disabled_context(
            signer.public_key().as_ref().try_into().unwrap(),
            evidence.clone(),
        )
        .unwrap();
    let binding = context.binding();
    assert_eq!(binding["policy"], DISABLED_POLICY);
    assert_eq!(binding["boundary"]["textEstimatePicoUsd"], 10_240_000u64);
    assert_eq!(binding["boundary"]["allowanceUsdCents"], 500);
    assert_eq!(binding["boundary"]["hardBillingCap"], false);
    assert_eq!(binding["liveAuthority"], false);
    let request = context
        .request(
            "FAKE_OWNER_ONLY_SECRET",
            "https://openrouter.ai/api/v1/chat/completions"
                .parse()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(request.method(), reqwest::Method::POST);
    assert_eq!(
        request.url().as_str(),
        "https://openrouter.ai/api/v1/chat/completions"
    );
    assert_eq!(
        request.body().unwrap().as_bytes().unwrap(),
        serde_json::to_vec(&plan.body(&plan.input).unwrap()).unwrap()
    );
    assert!(!format!("{request:?}").contains("FAKE_OWNER_ONLY_SECRET"));
    for url in [
        "http://openrouter.ai/api/v1/chat/completions",
        "https://openrouter.ai/api/v1/chat/completions?x=1",
        "https://other.invalid/api/v1/chat/completions",
        "http://127.0.0.1:1/model/chat/completions",
    ] {
        assert!(context
            .request("FAKE_OWNER_ONLY_SECRET", url.parse().unwrap())
            .is_err());
    }
    for seed in [None, Some([42; 32])] {
        let key = seed
            .map(|s| {
                Ed25519KeyPair::from_seed_unchecked(&s)
                    .unwrap()
                    .public_key()
                    .as_ref()
                    .try_into()
                    .unwrap()
            })
            .unwrap_or(TEST_PUBLIC_KEY);
        assert!(plan.disabled_context(key, evidence.clone()).is_err());
    }
    let mut costly = evidence.clone();
    costly.prompt_pico_usd = 1_000_000_000;
    assert!(plan
        .disabled_context(signer.public_key().as_ref().try_into().unwrap(), costly)
        .is_err());
    let mut missing = evidence;
    missing.terms_sha256.clear();
    assert!(plan
        .disabled_context(signer.public_key().as_ref().try_into().unwrap(), missing)
        .is_err());
}

#[test]
fn disabled_boundary_credentials_are_bounded_native_file_only_and_fail_redacted() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.amp/in/credential-fixtures")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    let path = root.as_path().join(".env.local");
    let model = "~deepseek/deepseek-v4-flash-latest";
    let source = CredentialFile::new(path.clone(), model.into()).unwrap();
    assert_eq!(
        source.settings_sha(),
        "7437607f1b77652a74190279783d214c20b907a6dd3d7d84278850752b06a4c3"
    );
    for (text, valid) in [
        (format!("LLM_API_KEY=FAKE_OWNER_ONLY_SECRET\nLLM_MODEL={model}\n"),true),
        (format!("export LLM_API_KEY = 'FAKE_OWNER_ONLY_SECRET' # retained comment\nLLM_MODEL=\"{model}\"\nOTHER=value\n"),true),
        (format!("LLM_API_KEY=FAKE_OWNER_ONLY_SECRET\nLLM_MODEL={model}\nLLM_PROVIDER=openrouter\nLLM_BASE_URL=https://openrouter.ai/api/v1\n"),true),
        ("LLM_API_KEY=FAKE_OWNER_ONLY_SECRET\nLLM_MODEL=changed\n".to_string(),false),
        (format!("LLM_MODEL={model}\n"),false),
        (format!("LLM_API_KEY=****masked\nLLM_MODEL={model}\n"),false),
        (format!("LLM_API_KEY=`FAKE_OWNER_ONLY_SECRET`\nLLM_MODEL={model}\n"),false),
        (format!("LLM_API_KEY=FAKE_OWNER_ONLY_SECRET\nLLM_MODEL={model}\nLLM_PROVIDER=other\n"),false),
        (format!("LLM_API_KEY=FAKE_OWNER_ONLY_SECRET\nLLM_MODEL={model}\nLLM_BASE_URL=http://localhost:1\n"),false),
        (format!("LLM_API_KEY=FAKE_OWNER_ONLY_SECRET\nLLM_MODEL={model}\nLLM_MODEL={model}\n"),false),
        (format!("LLM_API_KEY=\"FAKE_OWNER_ONLY_SECRET\\nBAD\"\nLLM_MODEL={model}\n"),false),
        (format!("LLM_API_KEY='FAKE_OWNER_ONLY_SECRET\nLLM_MODEL={model}\n"),false),
        ("X".repeat(65537),false),
    ] {
        std::fs::write(&path,text).unwrap();
        std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o600)).unwrap();
        let result = source.read();
        assert_eq!(result.is_ok(),valid);
        if let Ok(key) = result { assert_eq!(key,"FAKE_OWNER_ONLY_SECRET"); }
        else { assert_eq!(result.unwrap_err(),"model_credential_unavailable"); }
    }
    std::fs::write(
        &path,
        format!("LLM_API_KEY=FAKE_OWNER_ONLY_SECRET\nLLM_MODEL={model}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(source.read().is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let alias = root.join("alias");
    std::fs::hard_link(&path, &alias).unwrap();
    assert!(source.read().is_err());
    std::fs::remove_file(alias).unwrap();
    assert!(source.read().is_ok());
    std::fs::write(&path, [0xff, 0xfe]).unwrap();
    assert!(source.read().is_err());
    std::fs::write(
        &path,
        format!("LLM_API_KEY=FAKE_OWNER_ONLY_SECRET\nLLM_MODEL={model}\n"),
    )
    .unwrap();
    assert!(source.read().is_ok());
    let real = root.as_path().join("real");
    std::fs::rename(&path, &real).unwrap();
    std::os::unix::fs::symlink(&real, &path).unwrap();
    assert!(source.read().is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(source.read().is_err());
    std::fs::remove_dir(&path).unwrap();
    assert!(source.read().is_err());
    assert!(CredentialFile::new(root.as_path().join("other"), model.into()).is_err());
    assert!(CredentialFile::new(".env.local".into(), model.into()).is_err());
    use std::os::unix::ffi::OsStrExt;
    let fifo = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(source.read().is_err());
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
