//! Read-only native owner view of a separately supervised frozen Journal.

use super::{policy, require, Journal, Run};
use rusqlite::OpenFlags;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Cannot receive commands, resume a Journal or mutate scoped run storage.
/// A snapshot alone is not approval or dispatch authority.
pub(crate) struct FrozenOwnerCheckpoint {
    path: PathBuf,
    run: Run,
    context: policy::Rehearsal,
}
impl FrozenOwnerCheckpoint {
    pub(crate) fn read(path: &Path, context: policy::Rehearsal) -> Result<Self, String> {
        if super::storage::is_scoped() {
            return Err("owner_checkpoint_owner_process_required".into());
        }
        require(path.is_absolute() && path.canonicalize().ok().as_deref() == Some(path))?;
        let connection = super::storage::open_sqlite(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| "owner_checkpoint_unavailable")?;
        connection
            .busy_timeout(Duration::from_millis(100))
            .map_err(|_| "owner_checkpoint_unavailable")?;
        let (run, _) = Journal::read_run(&connection)?;
        let binding = context.binding();
        require(
            run.state == "frozen"
                && run.owner_live == Some(true)
                && run.consumptions == 0
                && run.credential_lookups == 0
                && run.counters()["model"] == 0
                && run.bindings["modelRehearsalPlan"]["nativePrepared"] == true
                && context.disabled_hosted()
                && ["offlineFixture", "offlineHttp", "offlineResponse"]
                    .iter()
                    .all(|key| binding.get(*key).is_none())
                && run.bindings["modelRehearsal"] == binding,
        )?;
        Ok(Self {
            path: path.into(),
            run,
            context,
        })
    }

    pub(crate) fn report(&self) -> Value {
        self.run.report()
    }

    /// Burns the in-memory claimed capability even on failure. The durable
    /// intent tombstone is never removed, and this method never publishes or
    /// consumes approval or enables a transport.
    pub(crate) fn issue(
        self,
        directory: &Path,
        proposal: &super::owner::ExecutionProposal,
        claimed: super::owner::ClaimedOwnerExecutionIntent,
        plan: &policy::RehearsalPlan,
    ) -> Result<policy::Approval, String> {
        let current = Self::read(&self.path, self.context)?;
        require(current.report() == self.run.report())?;
        claimed.validate_frozen_report(proposal, &current.report(), plan, &current.context)?;
        let intent = claimed.binding()?;
        let payload = serde_json::json!({"version":1,"runId":current.run.run_id,
            "checkpoint":current.run.checkpoint,
            "scopeSha256":super::sha(&serde_json::to_vec(&policy::scope(&current.run)).unwrap()),
            "nonce":intent["nonce"],"expiresUnixSeconds":intent["expiresUnixSeconds"]});
        policy::validate_claims(&payload, &current.run)?;
        let approval = current.context.sign_owner_payload(directory, payload)?;
        approval.verify(&current.run, Some(&current.context))?;
        Ok(approval)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::mock_run_authority::{policy, Command, Journal};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde_json::{json, Value};

    #[test]
    fn owner_checkpoint_issues_only_for_its_claimed_unchanged_journal() {
        use crate::infra::mock_run_authority::owner::{self, ExecutionKind, ExecutionProposal};
        use std::os::unix::fs::PermissionsExt;
        for damage in ["none", "stale", "intent-binding", "signer"] {
            let root =
                std::env::temp_dir().join(format!("owner-checkpoint-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let root = root.canonicalize().unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            let private = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
            let signer = Ed25519KeyPair::from_pkcs8(private.as_ref()).unwrap();
            for (name, bytes) in [
                ("approval-signing-key.pk8", private.as_ref()),
                ("approval-verifier.ed25519", signer.public_key().as_ref()),
            ] {
                let file = root.join(name);
                std::fs::write(&file, bytes).unwrap();
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            let plan: policy::RehearsalPlan = serde_json::from_value(json!({"nativePrepared":true,
                "model":"~deepseek/deepseek-v4-flash-latest","observedTarget":"deepseek/deepseek-v4-flash-0731",
                "route":"wafer/us","provider":"Wafer",
                "input":{"artist":"Public Artist","album":"Public Album","tracks":["First","Different"]}
            })).unwrap();
            let pin = super::super::sha(signer.public_key().as_ref());
            let context = || {
                plan.disabled_context(
                    signer.public_key().as_ref().try_into().unwrap(),
                    policy::boundary::TextPricing {
                        prompt_pico_usd: 1,
                        completion_pico_usd: 2,
                        catalog_sha256: super::super::sha(b"public catalog"),
                        terms_sha256: super::super::sha(b"public terms"),
                    },
                )
                .unwrap()
            };
            let membership: Value = serde_json::from_str(include_str!(
                "../../../test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json"
            ))
            .unwrap();
            let mut bindings = json!({"executable":owner::current_executable_sha256().unwrap(),
                "modelRehearsalPlan":plan,"sourcesAndTools":[{"path":"public/test-fixture.rs",
                    "sha256":super::super::sha(b"public fixture")} ]});
            let proposal = ExecutionProposal::construct(
                ExecutionKind::OfflineRehearsal,
                &uuid::Uuid::new_v4().to_string(),
                &membership,
                &bindings,
                &plan,
                &context(),
                &pin,
            )
            .unwrap();
            let now = policy::now().unwrap();
            let intent = owner::sign_owner_intent(
                &root,
                &proposal,
                &uuid::Uuid::new_v4().to_string(),
                now,
                now + 1200,
                false,
            )
            .unwrap();
            let claimed = owner::verify_owner_intent(
                &root,
                &proposal,
                &intent,
                ExecutionKind::OfflineRehearsal,
                &pin,
            )
            .unwrap()
            .claim_once(&root)
            .unwrap();
            bindings["ownerExecution"] = claimed.binding().unwrap();
            if damage == "intent-binding" {
                bindings["ownerExecution"]["nonce"] = json!(uuid::Uuid::new_v4().to_string());
            }
            let path = root.join("registry.sqlite");
            let request = context().binding()["requestSha256"]
                .as_str()
                .unwrap()
                .to_owned();
            let (journal, handle) =
                Journal::create_rehearsal(&path, membership, bindings, context()).unwrap();
            journal
                .command(
                    &handle,
                    Command::Lease {
                        role: "owner".into(),
                    },
                )
                .unwrap();
            super::super::tests::freeze_request(&journal, &handle, Some(request));
            let checkpoint = FrozenOwnerCheckpoint::read(&path, context()).unwrap();
            if damage == "stale" {
                assert!(journal.command("not owner", Command::Freeze).is_err());
            }
            if damage == "signer" {
                std::fs::remove_file(root.join("approval-signing-key.pk8")).unwrap();
            }
            let before = std::fs::read(&path).unwrap();
            let result = checkpoint.issue(&root, &proposal, claimed, &plan);
            assert_eq!(
                std::fs::read(&path).unwrap(),
                before,
                "issuance must not consume or mutate the run"
            );
            assert_eq!(journal.report().unwrap()["credentialLookups"], 0);
            assert_eq!(journal.report().unwrap()["counters"]["model"], 0);
            if damage == "none" {
                let approval = result.unwrap();
                assert_eq!(approval.payload["nonce"], intent.nonce);
                assert_eq!(
                    approval.payload["expiresUnixSeconds"],
                    intent.expires_unix_seconds
                );
                journal
                    .command(&handle, Command::Approve { approval })
                    .unwrap();
                assert_eq!(journal.report().unwrap()["consumptions"], 1);
            } else {
                assert!(result.is_err(), "{damage}");
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn owner_checkpoint_reads_only_a_current_frozen_nonfixture_run() {
        let root = std::env::temp_dir().join(format!("owner-checkpoint-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let key = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
        let plan: policy::RehearsalPlan = serde_json::from_value(json!({"nativePrepared":true,
            "model":"~deepseek/deepseek-v4-flash-latest","observedTarget":"deepseek/deepseek-v4-flash-0731",
            "route":"wafer/us","provider":"Wafer",
            "input":{"artist":"Public Artist","album":"Public Album","tracks":["First","Different"]}
        })).unwrap();
        let context = || {
            plan.disabled_context(
                key.public_key().as_ref().try_into().unwrap(),
                policy::boundary::TextPricing {
                    prompt_pico_usd: 1,
                    completion_pico_usd: 2,
                    catalog_sha256: super::super::sha(b"public catalog"),
                    terms_sha256: super::super::sha(b"public terms"),
                },
            )
            .unwrap()
        };
        let membership: Value = serde_json::from_str(include_str!(
            "../../../test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json"
        ))
        .unwrap();
        let path = root.join("registry.sqlite");
        let request = context().binding()["requestSha256"]
            .as_str()
            .unwrap()
            .to_owned();
        let (journal, owner) = Journal::create_rehearsal(&path, membership,
            json!({"executable":super::super::sha(b"public test executable"),"modelRehearsalPlan":plan}), context()).unwrap();
        journal
            .command(
                &owner,
                Command::Lease {
                    role: "owner".into(),
                },
            )
            .unwrap();
        assert!(FrozenOwnerCheckpoint::read(&path, context()).is_err());
        super::super::tests::freeze_request(&journal, &owner, Some(request));
        let before = std::fs::read(&path).unwrap();
        let checkpoint = FrozenOwnerCheckpoint::read(&path, context()).unwrap();
        assert_eq!(checkpoint.report(), journal.report().unwrap());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "owner reads must not mutate authority"
        );
        let wrong_key = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let wrong_key = Ed25519KeyPair::from_pkcs8(wrong_key.as_ref()).unwrap();
        let wrong_context = plan
            .disabled_context(
                wrong_key.public_key().as_ref().try_into().unwrap(),
                policy::boundary::TextPricing {
                    prompt_pico_usd: 1,
                    completion_pico_usd: 2,
                    catalog_sha256: super::super::sha(b"public catalog"),
                    terms_sha256: super::super::sha(b"public terms"),
                },
            )
            .unwrap();
        assert!(FrozenOwnerCheckpoint::read(&path, wrong_context).is_err());
        assert!(journal.command("not owner", Command::Freeze).is_err());
        assert!(FrozenOwnerCheckpoint::read(&path, context()).is_err());
        assert_eq!(journal.report().unwrap()["consumptions"], 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
