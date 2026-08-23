# Soundrobe web service configuration

The headless server requires a public origin and one operator password before
it starts. The password-file secret takes precedence when both password
settings are present:

~~~text
SOUNDROBE_LISTEN_ADDR=0.0.0.0:8080
SOUNDROBE_PUBLIC_URL=https://soundrobe.example.com
SOUNDROBE_DATA_DIR=/config
SOUNDROBE_LIBRARY_ROOT_DIR=/libraries
SOUNDROBE_AUTH_PASSWORD_FILE=/run/secrets/soundrobe_password
# SOUNDROBE_AUTH_PASSWORD is the documented fallback when no file is used.
~~~

SOUNDROBE_PUBLIC_URL must be an absolute http:// or https:// origin without a
path, query, or credentials. It is used for same-origin mutation checks and
cookie policy. HTTPS origins receive the Secure cookie attribute; direct HTTP
should only be used on a trusted network while TLS terminates at a reverse
proxy for normal deployments.

The server fails startup when neither password source is configured, when the
password file cannot be read, or when the public origin is invalid. Passwords
are not logged or returned in API response bodies. Successful logins create
in-memory, 24-hour HttpOnly/SameSite=Strict sessions; restarting the server
invalidates them.

The service discovers only immediate directories under
SOUNDROBE_LIBRARY_ROOT_DIR. The server path-confinement layer is ready for
browser operations: it canonicalizes supplied paths against their originating
mount and rejects traversal, symlink escapes, and cross-root moves.

SIGINT and SIGTERM initiate graceful shutdown: new API requests are rejected
and active queued writes are allowed to drain before the process exits.
