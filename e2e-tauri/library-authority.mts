import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { cleanupE2eWorkspace, prepareE2eWorkspace } from "./fixtures.ts";

if (process.platform !== "darwin" && process.platform !== "linux") {
  console.log("Library authority desktop checks run on macOS and Linux.");
  process.exit(0);
}

assert.equal(process.env.SOUNDROBE_E2E_MANIFEST, undefined, "requires a new test workspace");
const binary = path.resolve(
  "src-tauri/target", process.argv.includes("--debug") ? "debug" : "release", "soundrobe",
);
fs.accessSync(binary, fs.constants.X_OK);
const { root, manifest } = prepareE2eWorkspace();
const profile = path.join(root, "home", ".soundrobe");
assert.equal(process.env.HOME, path.join(root, "home"), "requires an isolated desktop profile");

const track = manifest.workflowTrack;
const trackBefore = fs.readFileSync(track);
const art = path.join(manifest.convertAlbum, "artist.jpg");
const artBefore = Buffer.from("artist artwork before the fixture write");
const artAfter = Buffer.from("artist artwork after the fixture write");
const digest = (bytes: Buffer) => createHash("sha256").update(bytes).digest("hex");
const profileId = () => fs.readFileSync(path.join(profile, "library-authority-profile-id"), "utf8").trim();

function run(phase: string) {
  execFileSync(path.resolve("node_modules/.bin/wdio"), ["run", "wdio.conf.ts"], {
    env: { ...process.env, SOUNDROBE_E2E_AUTHORITY_PHASE: phase, SOUNDROBE_E2E_BINARY: binary },
    stdio: "inherit",
    timeout: 150_000,
  });
}

function withDb<T>(action: (db: DatabaseSync) => T): T {
  const db = new DatabaseSync(path.join(profile, "worker.db"));
  try {
    return action(db);
  } finally {
    db.close();
  }
}

function expectRecovery(write: string, reversal: string) {
  withDb((db) => {
    assert.equal(db.prepare("SELECT state FROM write_intents WHERE run_id=?")
      .get("fixture-interrupted::write")?.state, write);
    assert.equal(db.prepare("SELECT status FROM reversal_manifests WHERE revision=?")
      .get("fixture-reversal")?.status, reversal);
  });
}

// Seed a process-exit checkpoint using only the isolated, real application DB.
function seedInterruption(owner: string) {
  const copiedRoot = fs.realpathSync(manifest.library);
  const marker = JSON.parse(fs.readFileSync(path.join(copiedRoot, ".soundrobe-library-authority"), "utf8"));
  assert.equal(marker.ownerProfileId, owner);
  const identity = [marker.libraryId, owner];
  const trackPath = fs.realpathSync(track);
  const album = fs.realpathSync(manifest.convertAlbum);
  const artPath = path.join(album, "artist.jpg");
  const removedCover = path.join(album, "already-removed.jpg");
  const backups = path.join(fs.realpathSync(profile), "write-recovery");
  fs.mkdirSync(backups, { mode: 0o700, recursive: true });
  const trackBackup = path.join(backups, "fixture-track.bak");
  const artBackup = path.join(backups, "fixture-artwork.bak");
  fs.writeFileSync(trackBackup, trackBefore, { mode: 0o600 });
  fs.writeFileSync(artBackup, artBefore, { mode: 0o600 });
  fs.writeFileSync(artPath, artAfter);
  const now = Math.floor(Date.now() / 1000);

  withDb((db) => {
    db.exec("BEGIN");
    db.prepare(`INSERT INTO write_intents
      (run_id,path,before_sha256,backup_path,patch,state,updated_at)
      VALUES (?,?,?,?,?,'applying',?)`)
      .run("fixture-interrupted::write", trackPath, digest(trackBefore), trackBackup,
        JSON.stringify({ title: "Interrupted fixture" }), now);
    db.prepare(`INSERT INTO authority_write_bindings
      (run_id,path,library_id,owner_profile_id,relative_path) VALUES (?,?,?,?,?)`)
      .run("fixture-interrupted::write", trackPath, marker.libraryId, owner, path.relative(copiedRoot, trackPath));
    const files = [
      { path: removedCover, target: removedCover, before: null, after: digest(Buffer.from("generated cover")),
        backup: null, original_mode: null, phase: "restoring", authority: identity },
      { path: artPath, target: artPath, before: digest(artBefore), after: digest(artAfter),
        backup: artBackup, original_mode: 0o640, phase: "pending", authority: identity },
    ];
    db.prepare(`INSERT INTO reversal_manifests
      (revision,album_path,manifest,status,updated_at) VALUES (?,?,?,'pending',?)`)
      .run("fixture-reversal", album, JSON.stringify({ files }), now);
    for (const file of files) {
      db.prepare("INSERT INTO review_target_owners (path,revision) VALUES (?,?)")
        .run(file.target, "fixture-reversal");
    }
    db.exec("COMMIT");
  });
}

try {
  run("unmarked");
  assert.deepEqual(fs.readFileSync(track), trackBefore);
  const owner = profileId();
  execFileSync(binary, ["--nominate-library", manifest.library], { stdio: "inherit" });
  run("owner-edit");
  const edited = fs.readFileSync(track);
  assert.notDeepEqual(edited, trackBefore);
  seedInterruption(owner);
  expectRecovery("applying", "pending");

  const saved = path.join(root, "nominated-profile");
  fs.renameSync(profile, saved);
  fs.mkdirSync(profile);
  // A fresh profile has a different identity even with the same copied checkpoint.
  fs.copyFileSync(path.join(saved, "worker.db"), path.join(profile, "worker.db"));
  run("foreign");
  assert.notEqual(profileId(), owner);
  expectRecovery("applying", "pending");
  assert.deepEqual(fs.readFileSync(track), edited);
  assert.deepEqual(fs.readFileSync(art), artAfter);

  fs.renameSync(profile, path.join(root, "foreign-profile"));
  fs.renameSync(saved, profile);
  run("owner-restart");
  assert.equal(profileId(), owner);
  expectRecovery("kept_current", "completed");
  assert.deepEqual(fs.readFileSync(track), edited);
  assert.deepEqual(fs.readFileSync(art), artBefore);
  assert.equal(fs.statSync(art).mode & 0o777, 0o640);
  console.log("Two-profile desktop authority and restart recovery passed.");
} finally {
  cleanupE2eWorkspace(root);
}
