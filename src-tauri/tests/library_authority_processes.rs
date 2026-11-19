//! Exercise write authority through the shipped server entry point, with a
//! local copied library and two independent profiles.
#![cfg(all(feature = "server", any(target_os = "macos", target_os = "linux")))]

use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use soundrobe_lib::state::{
    library_authority::{acquire, load_or_create_profile_id},
    worker::WorkerStore,
};
use std::{
    collections::BTreeMap,
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

const CHANGED: &str = "Artist/Album/01.mp3";
const UNCHANGED: &str = "Artist/Album/02.mp3";

struct Fixture {
    root: PathBuf,
    client: Client,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("soundrobe-authority-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let fixture = Self {
            root: root.canonicalize().unwrap(),
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
        };
        fs::create_dir_all(fixture.library().join("Artist/Album")).unwrap();
        let original = include_bytes!("../../test/fixtures/tauri/media-corpus/minimal.mp3");
        for track in [CHANGED, UNCHANGED] {
            fs::write(fixture.library().join(track), original).unwrap();
        }
        fs::create_dir(fixture.root.join("web")).unwrap();
        for profile in ["owner", "foreign"] {
            let dir = fixture.root.join(profile);
            fs::create_dir(&dir).unwrap();
            fs::write(
                dir.join("background-tagging.json"),
                json!({
                    "settings": {
                        "enabled": false, "schedule": "0 0 * * *", "directories": [],
                        "watchEnabled": false, "skipUnchanged": true
                    },
                    "runs": []
                })
                .to_string(),
            )
            .unwrap();
        }
        fixture
    }

    fn library(&self) -> PathBuf {
        self.root.join("libraries/Music")
    }

    fn command(&self, profile: &str, origin: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_soundrobe-server"));
        command
            .env_clear()
            .env("SOUNDROBE_PUBLIC_URL", origin)
            .env("SOUNDROBE_DATA_DIR", self.root.join(profile))
            .env("SOUNDROBE_LIBRARY_ROOT_DIR", self.root.join("libraries"))
            .env("SOUNDROBE_WEB_ROOT", self.root.join("web"))
            .env("AUTO_TAG_REMOTE_LOOKUP", "false")
            .env("AUTO_TAG_DISCOGS_ENABLED", "false");
        command
    }

    async fn start(&self, profile: &str) -> Server {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let origin = format!("http://{addr}");
        let log = self.root.join(format!("{profile}.log"));
        let mut command = self.command(profile, &origin);
        command
            .env("SOUNDROBE_LISTEN_ADDR", addr.to_string())
            .stdout(Stdio::null())
            .stderr(fs::File::create(&log).unwrap());
        drop(listener);
        let mut server = Server {
            child: command.spawn().unwrap(),
            origin,
            client: self.client.clone(),
        };
        for _ in 0..100 {
            if self
                .client
                .get(format!("{}/healthz", server.origin))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                return server;
            }
            assert!(
                server.child.try_wait().unwrap().is_none(),
                "server exited: {}",
                fs::read_to_string(&log).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "server never became ready: {}",
            fs::read_to_string(log).unwrap()
        );
    }

    fn worker(&self, profile: &str) -> WorkerStore {
        let worker = WorkerStore::new_in(self.root.join(profile));
        assert!(worker.initialize());
        worker
    }

    fn intent_states(&self, profile: &str) -> BTreeMap<String, String> {
        self.worker(profile)
            .reconcile_write_intents()
            .unwrap()
            .into_iter()
            .map(|entry| (entry.intent.run_id, entry.intent.state))
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Server {
    child: Child,
    origin: String,
    client: Client,
}

impl Server {
    async fn can_browse(&self) {
        let response = self
            .client
            .get(format!("{}/api/v1/libraries", self.origin))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.text().await.unwrap().contains("\"Music\""));
    }

    async fn write(&self, track: &Path, title: &str) -> (StatusCode, String) {
        let response = self
            .client
            .post(format!("{}/api/v1/commands/track%3Awrite", self.origin))
            .header("Origin", &self.origin)
            .json(&json!({"path":track,"fields":{"title":title}}))
            .send()
            .await
            .unwrap();
        (response.status(), response.text().await.unwrap())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn two_server_profiles_share_browse_access_but_only_owner_writes_and_recovers() {
    let fixture = Fixture::new();
    let library = fixture.library();
    let changed = library.join(CHANGED);
    let unchanged = library.join(UNCHANGED);
    let before = fs::read(&changed).unwrap();
    let owner = fixture.start("owner").await;
    let foreign = fixture.start("foreign").await;
    owner.can_browse().await;
    foreign.can_browse().await;
    let (status, error) = owner.write(&changed, "Unmarked edit").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
    assert!(error.contains("not been nominated"), "{error}");
    assert_eq!(fs::read(&changed).unwrap(), before);

    let nominated = fixture
        .command("owner", &owner.origin)
        .arg("--nominate-library")
        .arg(&library)
        .output()
        .unwrap();
    assert!(nominated.status.success(), "{nominated:?}");
    let owner_id = load_or_create_profile_id(&fixture.root.join("owner")).unwrap();
    let foreign_id = load_or_create_profile_id(&fixture.root.join("foreign")).unwrap();
    assert_ne!(owner_id, foreign_id);
    let (status, error) = foreign.write(&changed, "Foreign edit").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
    assert!(error.contains("different writer profile"), "{error}");
    assert_eq!(fs::read(&changed).unwrap(), before);
    let (status, response) = owner.write(&changed, "Nominated edit").await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(
        serde_json::from_str::<Value>(&response).unwrap()["title"],
        "Nominated edit"
    );
    let edited = fs::read(&changed).unwrap();
    assert_ne!(edited, before);
    drop(owner);
    drop(foreign);

    // Stage the checkpoints a restarted executable must interpret. Use the
    // native worker for backups and authority bindings; simulate the changed
    // media landing before its receipt can be recorded.
    fs::write(&changed, &before).unwrap();
    let lease = acquire(&library, owner_id).unwrap();
    let worker = fixture.worker("owner");
    for (run, relative) in [("hold::write", CHANGED), ("retry::write", UNCHANGED)] {
        let track = library.join(relative);
        worker
            .prepare_write_intent(&track, run, &json!({"title":"Nominated edit"}))
            .unwrap();
        worker
            .bind_write_authority(&track, run, &lease, Path::new(relative))
            .unwrap();
        worker.begin_write_intent(&track, run).unwrap();
    }
    fs::write(&changed, &edited).unwrap();
    drop(worker);
    drop(lease);
    // Give the second profile the same checkpoint, but retain its different ID.
    let owner_db = fixture.root.join("owner/worker.db");
    fs::copy(owner_db, fixture.root.join("foreign/worker.db")).unwrap();
    let pending = BTreeMap::from([
        ("hold::write".into(), "applying".into()),
        ("retry::write".into(), "applying".into()),
    ]);
    assert_eq!(fixture.intent_states("owner"), pending);
    let _foreign = fixture.start("foreign").await;
    assert_eq!(fixture.intent_states("foreign"), pending);
    let owner = fixture.start("owner").await;
    assert_eq!(
        fixture.intent_states("owner"),
        BTreeMap::from([
            ("hold::write".into(), "needs_recovery".into()),
            ("retry::write".into(), "retry_safe".into()),
        ])
    );
    assert_eq!(fs::read(&changed).unwrap(), edited);
    assert_eq!(fs::read(&unchanged).unwrap(), before);
    let (status, error) = owner.write(&changed, "Cannot bypass recovery").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(fs::read(&changed).unwrap(), edited);
    let (status, response) = owner.write(&unchanged, "Safe retry").await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(
        serde_json::from_str::<Value>(&response).unwrap()["title"],
        "Safe retry"
    );
    assert_ne!(fs::read(unchanged).unwrap(), before);
}
