# Soundrobe web service

The web runtime uses the same React interface and Rust metadata pipeline as
the desktop app. It is a single-user, single-process service for a trusted
operator behind an HTTPS reverse proxy.

## Local container

Build the image and create a password file without committing either file:

```sh
printf '%s\n' 'replace-with-a-long-password' > /absolute/path/soundrobe-password
chmod 600 /absolute/path/soundrobe-password
docker build -t soundrobe:local .
```

Copy `.env.web.example` to `.env.web`, set the public URL, password-file path,
and at least one library bind path, then validate and start the Compose stack:

```sh
docker compose --env-file .env.web -f docker-compose.yml config --quiet
docker compose --env-file .env.web -f docker-compose.yml up -d
curl --fail http://127.0.0.1:8080/healthz
```

The service listens on port `8080` in the container. `SOUNDROBE_PORT` changes
the host port; `SOUNDROBE_PUBLIC_URL` must be the browser-visible origin, not
the internal container address. Startup fails if the public URL or password is
missing.

## Libraries and permissions

Every library is mounted as a direct immediate child of `/libraries`. The
directory name becomes the browser-visible library name. Add one Compose bind
mount per root; do not mount a broad parent directory and rely on nested paths.
The container user must have read/write permission on `/config` and every
library bind mount. `SOUNDROBE_UID` and `SOUNDROBE_GID` let a NAS deployment
match the host ownership. The service rejects traversal, symlink escapes, and
cross-library moves.

`/config` contains the persistent configuration, cache, SQLite state, and
aliases. Back it up before upgrades. The container root filesystem is
read-only; only `/config`, library mounts, and the `/tmp` tmpfs are writable.

## Reverse proxy and SSE

Terminate TLS at Nginx Proxy Manager or another reverse proxy and proxy the
same origin to the container. Preserve cookies, allow long-lived
`/api/v1/events` connections, disable response buffering for that endpoint,
and use a read timeout longer than the expected assistant or audit operation.
No CORS API is exposed; browser mutations must remain same-origin.

The only public operational endpoint is `GET /healthz`. The application API
requires the opaque session cookie issued by the login endpoint. Do not expose
the container directly to the Internet without an HTTPS and access-control
boundary.

## Local development

For a local browser-service run, make a directory whose immediate children are
the libraries you want Soundrobe to expose. Set a temporary local password,
then start the server on loopback:

```sh
export SOUNDROBE_AUTH_PASSWORD='replace-with-a-local-password'
# Replace this with an existing directory that contains your library folders.
just web-local "/path/to/libraries-root"
```

The recipe builds the web assets and starts on port 8080. Open either
`http://127.0.0.1:8080` or `http://localhost:8080`; both loopback addresses are
accepted for local browser requests. By default it uses the standalone app's
`~/.soundrobe/` data directory, so the browser service uses the same local
settings and provider configuration during development. Set `SOUNDROBE_DATA_DIR`
to an isolated directory when testing configuration changes. You can also override
`SOUNDROBE_LISTEN_ADDR`, `SOUNDROBE_PUBLIC_URL`, or
`SOUNDROBE_ADDITIONAL_ORIGINS` when needed. Stop it with `Ctrl-C`. Do not use
this local HTTP setup as an Internet deployment; use the container setup with
an HTTPS reverse proxy instead.

## Updates and checks

The repository publishes `ghcr.io/ydeng11/soundrobe:<version>` and `:latest`
for version tags after an amd64 container smoke test. Manual workflow runs may
provide an explicit version and always include a commit-SHA tag. Publication
proves the image build and CI smoke test only; it does not verify a particular
NAS, reverse proxy, DNS, or HTTPS deployment.

Pull the desired image tag and recreate the service, preserving `/config` and
the library mounts:

```sh
docker compose --env-file .env.web -f docker-compose.yml pull
docker compose --env-file .env.web -f docker-compose.yml up -d
docker compose --env-file .env.web -f docker-compose.yml ps
```

A healthy container proves only that the process answers `/healthz`. It does
not prove NAS bind-mount permissions, DNS, reverse-proxy routing, HTTPS, or
public reachability. Verify those separately after each deployment.

## Background tagging

The service tags unattended as well as on demand. An operator configures it from
**Settings → Background tagging** and watches the result on **Monitoring**; both
are ordinary browser pages, so nothing here needs shell access.

### Schedule

Runs are driven by a five-field crontab (`minute hour day month weekday`)
evaluated in the container's local time, so `0 0 * * *` means local midnight and
not midnight UTC. The usual forms are supported: `*`, `*/n` steps, ranges
(`9-17`), lists (`15,45`), and both `0` and `7` for Sunday. When both day fields
are restricted, a date matches if *either* matches, which is standard cron
behaviour.

The preset list is the reference product's own: every 1, 3 and 10 minutes,
hourly, every 2 hours, daily at midnight, plus a custom expression. An expression that
cannot be parsed is refused with the reason and the stored schedule is left
alone, so a typo can never leave background tagging unable to run.

The scheduler evaluates every 20 seconds and fires each slot once. A slot whose
minute has already passed still runs late rather than being dropped, but only
one missed slot is caught up per evaluation, so a service that was stopped for a
week does not start a week of backlog. A due slot is retired as it is reported:
if another operation holds the tagging lock the occurrence is skipped and
logged, never queued behind it.

### What gets tagged

By default every mounted library is covered. Adding folder paths scopes runs to
those folders, and a configured folder that no longer exists is dropped rather
than failing the run. "Only tag releases with incomplete tags" (on by default)
skips a release whose tracks all carry a title, artist and album **and** a
provider identifier, which is the same test the startup sweep always used.

Enabling "Tag music as it appears in the covered folders" makes the service tag
newly added music. Detection uses each release folder's change time, so a
watched folder does not cause a full library rescan on every tick, and a run
started this way covers only the arrivals, not the library.

Turning watching on takes a baseline first: everything already present is
recorded as known, so switching the setting on does not queue the whole library.
Consequently a release that appears in the same moment watching is enabled is
treated as pre-existing; the next scheduled run still covers it because it is
untagged, and it is never tagged twice.

### Observing it

`~/.soundrobe/background-tagging.json` holds both the settings and the last 50
runs, and is the authoritative record of what happened. Monitoring renders it
alongside the live task queue. Its log source selector covers two files in the
service data directory:

| Source | File | Contents |
| --- | --- | --- |
| Application log | `auto-tagger.log` | What the service process did, including each scheduled start. |
| Auto-tag trace | `auto-tag-debug-YYYY-MM-DD.log` | Verbose per-release tracing, written only while debug logging is on. |

A source with no file yet reports that plainly instead of showing an empty log.
Background tagging is on by default because the service already swept mounted
libraries at startup before it was configurable; turning it off is one switch in
Settings.

### Reviewing what tagging could not place

A run that cannot apply an authoritative match leaves the album flagged rather
than guessing, and the Workspace is where those albums are meant to be fixed.
It names them above the file table and marks them `Review` in the folder tree,
both keyed by the service's own per-release classification, so the list is
whatever the last run or startup survey actually decided — the browser infers
nothing from tag contents.

The flag reflects that classification, not a promise that the album is still
broken: an album repaired by hand keeps its flag until the service classifies it
again, and a restart re-derives the list from a fresh startup survey. The
Monitoring page shows every classification, including the failures that the
review list deliberately leaves out, because a failed release is a problem to
retry rather than a tagging decision for a person.

Three tools act on whichever album is open, and all of them write through the
same Rust writer as the rest of the app. They are the desktop app's own tools,
not a reduced version:

| Tool | Scope | What it does | What it will not do |
| --- | --- | --- | --- |
| Search | The open album | Narrows a query by artist, album, year, country, format, catalog number and barcode; filters, sorts and pages the results; loads per-release track counts on request; then maps each local file to a provider track. | Write before the mapping step is confirmed, or write a file left unmapped. |
| Number | Every track in the open album | Writes sequential track numbers and a track total using the desktop's eight ordering rules. | Touch files outside the album. |
| Convert | The selection, or the whole album when nothing is selected | Rewrites tags from file names, one tag from another, or file names from tags, using the desktop's presets. | Run a pattern whose result equals the value already stored, or rename onto a file that already exists. |

Two details are worth knowing because they are the difference between a tool
that is safe to point at a library and one that is not:

- **Per-file failures are reported, not summarised away.** The writers return
  successes and failures together, so a partly completed run says which files it
  did not change instead of reporting success for the batch.
- **The mapping step is editable.** Each row pairs one local file with one
  provider track, and either side can be changed: re-point a file, edit the
  title, artist, track or disc number that will be written, or set the file to
  "Do not update" to leave it alone. Only the mapped files are written, and
  pointing two files at the same provider track is flagged before you apply.
