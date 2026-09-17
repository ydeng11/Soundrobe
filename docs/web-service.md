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
accepted for local browser requests. It stores local service state in
`.soundrobe-web-local/` by default; override `SOUNDROBE_DATA_DIR`,
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
