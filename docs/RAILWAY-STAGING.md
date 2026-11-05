# Railway staging

This is a staging configuration, not a production publication runbook. The
image in `deploy/Dockerfile` builds the API and admin UI separately, then runs
only the API as a non-root user. The UI is served from `/app`, with the API on
the same origin. `/health/ready` checks the database schema.

## Repositories

- The canonical repository is `nglmercer/RemakeBackend`. Changes land there
  through pull requests.
- The Railway project `robust-charisma` deploys the `RemakeBackend` service from
  a fork, `hernan-lc/RemakeBackend`. A change reaches Railway only after it is
  merged into the branch that service builds.
- The IaC file does not hardcode either repository. Set `RAILWAY_SOURCE_REPO` to
  the `owner/name` the service is connected to, and optionally
  `RAILWAY_SOURCE_BRANCH` (default `master`). `railway config plan` refuses to run
  without `RAILWAY_SOURCE_REPO`.

## Configuration

Set these in the Railway service's Variables tab. Never check credentials into Git.

- `DATABASE_URL`: the PostgreSQL connection string. Use a database dedicated to
  staging.
- `AUTH_SECRET`: random, at least 32 bytes.
- `APP_ORIGIN`: the public `https://` origin of the service.
- `PORT`: set by Railway. The API listens on `0.0.0.0:$PORT`. If `BIND_ADDR` is
  set, it must use the same port and must not be loopback.

At startup the API checks these values and refuses to start with an error that
names the variable to fix, for example `PORT is required on Railway`. `ADMIN_DIST_DIR`
is set by the image. Set a nonzero Railway deployment draining time so SIGTERM has
time to finish active requests. Review trusted proxy CIDRs before enabling
proxy-derived client IPs.

## Start command, pre-deploy command and healthcheck

- **Start command:** `bash /usr/local/bin/start-cloud`. It runs the API on every
  deployment and restart, and never applies migrations.
- **Pre-deploy command:** `cloud-api migrate`. It runs once per deployment,
  before the new container starts. A nonzero exit stops the deployment, so the
  previous version keeps serving.
- **Healthcheck:** `/health/ready`, with a 120 second timeout. A deployment is
  promoted only after this returns 200. It fails while migrations are missing,
  failed or changed.

## Diagnosing HTTP 502

Each startup step logs one line. Look for these in the deploy logs:

| Log message | Meaning |
| --- | --- |
| `configuration validated` | Environment checks passed |
| `startup mode` (`mode` = `server`, `migrate`, `worker`, `maintenance`) | Entry point that is running. `rejected` means the arguments were refused |
| `database connection established` | The database accepted a connection |
| `HTTP listener bound` (`address=0.0.0.0:<PORT>`) | The API is reachable |
| `migrations applied; exiting without starting the HTTP server` | Pre-deploy migration finished with exit 0 |

- **502 on an active deployment, with no `HTTP listener bound`:** startup stopped.
  Find the `startup configuration rejected` line, which names the variable to fix.
- **Healthcheck fails with 503 on `/health/ready`:** the database is reachable but
  migrations are missing or failed. Check that the pre-deploy migration printed
  `migrations applied`.
- **Pre-deploy log without `migrations applied`:** the migration did not finish,
  and the deployment should have stopped. Do not promote it by hand.
- **Idle service wakes slowly:** a sleeping service answers its first request
  after waking. A failed wake-up can look like a 502.

## Railway Infrastructure as Code

`.railway/railway.ts` declares the start command, pre-deploy command, healthcheck
and variable names. Variable values stay in Railway (`preserve()`). Config as
Code (`railway.toml`) is deprecated and stops being read on **2026-12-01**.

Roll out in this order, stopping at any unexpected change:

1. `railway login`, `railway link`, and `RAILWAY_SOURCE_REPO` set to the connected repository.
2. `railway config migrate status`: shows whether any service still reads Config
   as Code. If one does, run `railway config migrate cutover` first.
3. `railway config plan`: read-only. Continue only when the changes match this document.
4. `railway config apply`: changes the live service. Run it only with explicit
   approval, then deploy and check the startup logs above.
5. Remove `railway.toml` only after a clean plan and apply.

Rollback: `railway config migrate undo` restores the Config as Code paths saved by
a cutover. To revert a setting applied from `.railway/railway.ts`, restore the
previous file, run `railway config plan`, and apply only after reviewing it.

## Publication scanner requirement

The image includes ClamAV but does not start it by default. To run it alongside
the API, set `CLOUD_CLAMD_CONFIG` to the absolute path of a readable `clamd.conf`
and set `CLAMAV_ADDR` to its loopback IP and port. The config must run in the
foreground, bind only to loopback, use a separately provisioned and regularly
updated signature database, scan archives, alert when scan limits are exceeded,
and allow streams at least as large as the configured artifact limit. Both
processes run as UID 10001. If either exits, the supervisor stops the other and
the service manager can restart them. A separate Railway service is not reachable
through the loopback-only scanner interface. Without the optional scanner the
API can serve existing releases, but verification and publication workers cannot
complete. Provision and verify current signatures and resource limits before
publishing; the image does not update or assess freshness of the database.
The memory needed by real signatures has not been measured here and may exceed
a 1 GB total budget. ClamAV's INSTREAM protocol requires `StreamMaxLength` to
cover the upload size: https://docs.clamav.net/manual/Usage/ClamdProtocol.html

The image contains `/opt/cloud-clamd.conf.example` as a starting point. For that
example mount the volume at `/data`, create separate `/data/objects` and
`/data/clamav` directories readable by UID 10001, set
`STORAGE_LOCAL_DIR=/data/objects`, `CLOUD_CLAMD_CONFIG=/opt/cloud-clamd.conf.example`
and `CLAMAV_ADDR=127.0.0.1:3310`. Populate and update `/data/clamav` with
official signatures using a separately managed update process; the image does
not include an updater. The object directory must remain private and contain
only Cloud's own files. Measure scanner startup, updates, scan limits and peak
memory on the actual deployment before accepting this configuration.
Locally, `python3 deploy/test_start_cloud.py` checks that an unexpected scanner
exit stops the API. The example configuration was also exercised against a
local ClamAV instance with a synthetic signature; that does not validate the
container's ClamAV version or a production signature database.
With a disposable scanner and nine built staging packages, repeat the actual
application scanner check with:

```sh
CLOUD_CLAMAV_TEST_ADDR=127.0.0.1:3310 \
CLOUD_STAGING_PACKAGES_DIR=/path/to/nine-staging-packages \
cargo test -p cloud-api --locked real_clamav_scans_staging_packages -- --ignored
```

Migrations are deliberately explicit. Before an upgrade, take and verify a
database backup, stop incompatible writers, run `cloud-api migrate` using the
same image and configuration as the deployment, and confirm readiness. A
normal restart does not restore a backup or silently apply migrations. Restore
into a new isolated database, verify it, then explicitly cut over. Data and
artifact backups must be coordinated; the database alone is not sufficient.

## Local-volume backup and restore

Stop all API and worker processes that use the volume, along with direct
database writers and filesystem maintenance jobs. Every current Cloud
process configured with this local volume holds a shared advisory lock; the
backup and restore commands require the exclusive lock and fail when it is
held. This does not stop old binaries, direct SQL connections, other machines,
or filesystem tools, so operators must stop those separately. The filesystem
must support advisory locks and hard links correctly.

On Railway, a one-replica maintenance deployment can keep the volume mounted
without starting the API: configure the service's start command temporarily to
`sleep infinity`, deploy it, then use `railway ssh` to run the commands below.
First stop separate workers and direct writers. Store the backup outside the
volume and export it off the service before returning the start command to
`bash /usr/local/bin/start-cloud`; filesystem data outside the mounted volume is ephemeral. Ensure
the maintenance process has enough temporary disk space for the complete
backup. Test this procedure on a staging service before relying on it.

With the normal service variables set, run the following using a PostgreSQL
client at least as new as the server. `NEW_BACKUP_DIR` must not exist and must
be outside the volume. Run this on the machine where the volume is mounted.

```sh
cloud-api backup-local /secure-backups/new-backup
cloud-api verify-local-backup /secure-backups/new-backup
```

The command takes an exported repeatable-read PostgreSQL snapshot, creates a
custom-format `pg_dump`, copies local objects, and checks all current database
references against their stored size and digest. The manifest is written last;
an error can leave an incomplete directory without a valid manifest. It refuses
an existing destination and does not copy leftover `.tmp-*` files. Transfer an
encrypted copy off the volume, independently authenticate the backup, and keep
application secrets and registry signing keys separately. The manifest detects
accidental changes; it is not a cryptographic signature or encryption.

To restore, provision a **new, empty** PostgreSQL database and a new, empty
mode-0700 volume directory. Point `DATABASE_URL` and `STORAGE_LOCAL_DIR` to
those targets, stop all writers, and run:

```sh
cloud-api verify-local-backup /secure-backups/new-backup
cloud-api restore-local /secure-backups/new-backup
```

The command refuses a nonempty target database or volume, rechecks the files,
and runs `pg_restore` in a single database transaction. If restoration fails,
copied files can remain in the target volume and require inspection before a
retry; it never cleans or overwrites them automatically. Check readiness,
artifact digests, signed registry discovery, and application authentication on
the isolated target before cutover. The tools accept common libpq URL settings;
unsupported or duplicate URL query parameters fail closed. A local PostgreSQL
18 drill backed up and verified 86 objects from a published nine-release
staging registry, restored them into an empty database and volume, and passed
readiness and pinned-client installation for all nine releases. This used
disposable signing keys and a test scanner. pgrust's exported-snapshot support,
Railway volumes, offsite recovery, and production workloads remain unverified.

For a single replica, `STORAGE_LOCAL_DIR` selects a filesystem backend. Create
an existing private directory (mode 0700, writable by UID 10001) on a persistent
volume and set this to its absolute path. Configure either that directory or
all five S3 settings, never both. Uploads stream through the API and are checked
against the declared size and SHA-256 before atomic installation; completed
objects cannot be overwritten. The process must keep exclusive ownership of
the directory, and local storage is not suitable for multiple independent
replicas or a shared volume with untrusted writers. The app's `AUTH_SECRET`
also derives short-lived upload and inspection capabilities; rotation invalidates
outstanding URLs. Reverse proxies and access logs must not record their query
strings. Temporary files left by an interrupted process require operator review;
do not delete active objects.

The alternative S3 backend still requires a store with object versioning and
the documented conditional-write and checksum semantics. Railway's bucket
must not be substituted without verifying equivalent immutable storage.
Scanning, review, root trust, key management and promotion remain required. See `PRODUCTION-READINESS.md` and
`REGISTRY-OPERATIONS.md` for current gates.

The admin dependencies were installed with `npm ci --ignore-scripts` and the
build and 40 tests passed locally with Node 22.23.4, the version specified in
the image. The image and Railway deployment have not yet been exercised on Railway. Size,
CPU, memory, build dependencies, backups and real provider behavior must be
measured in staging. A 1 GB limit is a target, not a demonstrated capacity.

## Verify the staged official packages

After publishing a disposable staging generation containing the nine official
plugins, run the pinned TikTools registry client and installer against it. The
root key must come from an independent trusted channel, not from the registry:

```sh
CLOUD_STAGING_ORIGIN=http://127.0.0.1:5186 \
CLOUD_STAGING_GENERATION=1 \
CLOUD_STAGING_ROOT_PIN=/secure-path/staging-root.pub.json \
cargo test -p cloud-api --locked staging_official_packages_install_with_pinned_client -- --ignored
```

This downloads each package, verifies it using the pinned client and commits it
to an isolated temporary installation. It does not start the plugins or test
their external game, operating-system or network dependencies. A local staging
run passed for all nine packages; the scanner in that run used a synthetic test
signature, so it is not evidence of production antivirus coverage.
