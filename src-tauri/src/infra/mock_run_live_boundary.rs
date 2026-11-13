//! Disabled hosted-request preparation; no activation or signer provisioning.
use super::*;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

pub(crate) const DISABLED_POLICY: &str = "disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair";
pub(crate) const ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";

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
        (format!("LLM_API_KEY=FAKE_OWNER_ONLY_SECRET\nLLM_MODEL=changed\n"),false),
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
