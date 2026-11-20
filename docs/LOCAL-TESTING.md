# Local testing without external services

This is the fastest way to run every suite that works on a developer machine.
It needs no Docker, no ClamAV daemon, no staging registry and no network access.

## Run everything local

```sh
cargo run -p devtools -- db install   # once: installs the pgrust binary
cargo run -p devtools -- db up        # starts the local database
cargo remake test --all               # unit + integration, isolated databases
cargo remake db down                  # stops the local database when done
```

`cargo remake test --all` runs every non-ignored test. Ignored tests are skipped
by default; `--all` does not pass `--include-ignored`.

## What is skipped on purpose

Each ignored test needs an external input that only exists in staging or
production. Their local substitutes run in the default suite.

| Ignored test | Needs | Local substitute (runs by default) |
|---|---|---|
| `scanner::tests::real_clamav_scans_staging_packages` | a clamd daemon and nine staging `.plugin` packages (`CLOUD_CLAMAV_TEST_ADDR`, `CLOUD_STAGING_PACKAGES_DIR`) | `scanner::tests::nine_package_set_scans_clean_through_protocol_fixture` |
| `registry_tests::staging_official_packages_install_with_pinned_client` | a running staging registry (`CLOUD_STAGING_ORIGIN`, `CLOUD_STAGING_GENERATION`, `CLOUD_STAGING_ROOT_PIN`, `CLOUD_STAGING_PACKAGES_DIR`) | `registry_tests::nine_official_packages_publish_and_install_through_pinned_client` |
| `cloud_v2_tests::actual_app_sdk_epoch_contract_against_postgres` | the SDK test binary (`CLOUD_V2_APP_SDK_TEST_BIN`) | the same contract is exercised by the local integration suite; the binary is optional (see below) |
| `google_jwks_live_refresh_smoke` | live network access to Google | none; it is a live smoke test |

The real ClamAV and staging tests are for staging and production. They are not
needed to validate development work.

## Docker

Nothing in the local path uses Docker. `compose.yaml` only provides a database
for the containerised alternative. If Docker is missing, use `db up` above.

## Optional: run the SDK epoch test locally

The SDK binary is the `real_backend_epoch` target of the TikTools `cloud-client`
crate. Build it from the TikTools checkout and point the backend at it:

```sh
# from the TikTools checkout
cargo test -p cloud-client --test real_backend_epoch --no-run
# copy the path printed as "Executable ...", then from this repository:
CLOUD_V2_APP_SDK_TEST_BIN=/abs/path/to/real_backend_epoch-xxxx \
CLOUD_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:5432/cloud_test \
cargo test -p cloud-api actual_app_sdk_epoch_contract_against_postgres -- --ignored --test-threads=1
```

The test refuses any database that is not a loopback clone, so point
`CLOUD_TEST_DATABASE_URL` at the local database only.

## Enabling the production-only tests

Only when the staging or production resources exist, pass the variables above
and run the single test with `--ignored`. Do not run them in a shared database.
