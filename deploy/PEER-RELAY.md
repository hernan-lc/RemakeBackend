# WebRTC signaling on the existing Railway backend

The current target uses native Rust WebRTC DataChannels and signed SDP/ICE
through the existing `/api/v3/remote/socket` endpoint in `cloud-api`. Apply
`20261112000001_peer_directory.sql`, `20261112000002_rtc_signaling.sql` and
`20261112000003_relay_device_binding.sql` and
`20261112000004_native_relay_tunnels.sql`.
No separate Iroh relay service is part of the requested deployment.

Railway exposes public HTTP/TCP networking. Standard TURN allocations also
need a reachable relay network; client-side TURN-over-TCP does not by itself
solve the missing public UDP relay ports. Embedded TURN is **not implemented
or advertised** by this migration. The application WSS fallback's backend routing
and native automatic selection are implemented; the existing WSS endpoint is
not a TURN server. See [Railway networking limits](https://docs.railway.com/networking/public-networking/specs-and-limits).

The native Cloud host coordinator now registers its endpoint, refreshes confirmed
grants, receives signed negotiations and admits established DataChannels into
its local permission/receipt engine. Cloud builds of the normal Desktop/CLI supervisor and client now use this
transport. Version-three profiles pin the host device/key; version-two profiles
migrate only after the exact prior identity is admitted. Do not treat the current transport
laboratory as proof of NAT traversal between two physical PCs.

## Central ICE discovery configuration

Optionally set `RTC_STUN_URLS` to a comma-separated list of at most eight `stun:`
URLs on the API service, then restart it. The default is empty: no public STUN
provider is contacted implicitly. URLs cannot contain credentials, paths or
queries. Example: `RTC_STUN_URLS=stun:discovery.example.test:3478`; replace this
documentation hostname with a reachable service you operate or choose to use.

The authenticated `GET /api/v3/remote/rtc-config` returns protocol version 1 and
`stunUrls`. Both native roles fetch it automatically, and the host refreshes it
when reconnecting signaling. The existing `/peer-config` response stays compatible.
This setting does not start a STUN or TURN server in the API and does not provide
relay candidates. The 26-test isolated v3 suite covers authentication and exact
configured discovery; the native adapter separately tests real UDP STUN replies.

## Opaque application relay routing

The existing authenticated socket accepts `relay.open`, `relay.data` and
`relay.close`. Each side announces a temporary TLS CA signed by its registered
endpoint key and bound to the exact host, session, devices, negotiation UUID and
confirmed grant revision. Native TLS validates the peer; the backend does not
decrypt RPC or choose its business permissions. No PC certificate installation
or separate relay service is needed for this layer.

`relay.data` carries at most 16 KiB of opaque bytes encoded as base64url without
padding. Counters are independent per side and must be contiguous. Both live
socket generations and endpoint keys are pinned; every delivery rechecks the
active device/session/grant and current lease. Openings last 60 seconds; a host
answer extends the tunnel to at most eight hours from creation. There are at
most two live tunnels per session, 30 opens and 6000 records per device/minute,
and serialized tunnel enqueueing that refuses destinations with 64 pending
cross-node frames. Overflow while draining cross-node records closes the slow
connection rather than silently losing a TLS record. Local enqueueing reserves
capacity before committing and returns `peerBusy` when no slot is available.

Transient cross-node outboxes can hold public certificate announcements and TLS
records for delivery, with the existing 60-second retention. The tunnel table
stores only routing/counter metadata. A `relay.accepted` acknowledgment is not
a business receipt, and the backend never executes these opaque records.

The v3 suite verifies actual WebSocket delivery on one and two backend nodes,
bad signatures, foreign sessions, replay/gaps, oversize payloads, expiry,
revocation, socket replacement and concurrent bounded enqueueing. The native
layer separately tests mutual TLS over its record bridge. The normal SDK/host
coordinators now use that bridge after an unanswered eight-second direct attempt
or an explicit `directUnavailable` response. The real native acceptance fixture
also verifies automatic WSS fallback, scoped RPC/receipts and Cloud recovery.
Invalid device signatures or contexts never trigger fallback.

For diagnostics or a network policy that requires the application relay, set
`RTC_RELAY_ONLY=true` on the API. It defaults to false and accepts only `true`
or `false`. The PCs keep their ordinary configuration and receive the policy
response through authenticated signaling. Restore false to prefer direct ICE.
Capabilities announce this native fallback under
`remoteControl.rtcSignaling.applicationRelay`, explicitly separate from TURN.
If an active transport fails, Desktop reconnects the same profile with backoff
and recovers its projections/receipts. SDK and one-shot CLI callers reopen their
profile explicitly; no uncertain mutation is automatically replayed. A valid
controller whose host listener has not reconnected yet receives `peerOffline`,
which is temporary unavailability. A revoked grant or a missing/replaced sender
lease still fails authorization. Both sides' live lease generations remain
locked and checked before tunnel delivery.

The material below is historical evidence for the previous Iroh implementation,
not the deployment procedure for the WebRTC target. It will be retired when the
native cutover is complete.

## Previous native collaboration relay

`cloud-peer-relay` is an independent Iroh 1.4.1 service. It carries encrypted
endpoint traffic; it does not execute TikTools commands. It is not the existing
Cloud Teams WSS relay. Direct connections do not pass through this service.

## Configuration and deployment

1. Run the API's migrations, including `20261112000001_peer_directory.sql`.
2. Create a database login with SELECT permission on
   `cloud_remote_peer_endpoints`, `cloud_remote_devices`, `cloud_remote_hosts`
   and `cloud_remote_grants`. The relay requires no write permission. Supply its
   URL as `PEER_RELAY_DATABASE_URL`; do not commit the credential.
3. Issue a public certificate for a dedicated relay hostname. Set
   `PEER_RELAY_TLS_DIRECTORY` to the directory containing `fullchain.pem` and
   `privkey.pem`. The container runs as UID 10001; grant that UID read access.
   The private key must be a regular file with mode 0600. Deploy certificate
   renewals by replacing those files and restarting only the relay.
4. Publish TCP 443 and UDP 7842 for that hostname. Set the API's
   `PEER_RELAY_URLS=https://relay.example.com` (at most four comma-separated
   HTTPS origins), and restart the API to advertise it. Avoid an HTTP proxy/CDN
   unless it supports the Iroh relay connection upgrade and long-lived links.
5. Start `docker compose -f compose.peer.yaml up -d --build`. This definition
   deliberately does not manage the API or database. The relay must be able to
   reach the database through the URL supplied in step 2; container localhost
   is not the database host.

For a native process use `DATABASE_URL` and the `PEER_RELAY_*` variables in the
compose file. Loopback HTTP defaults to 127.0.0.1:8091; HTTPS to :8443 when the
two TLS files are supplied. Non-loopback plaintext HTTP requires the explicit
`PEER_RELAY_TRUST_TLS_PROXY=true` setting and a trusted TLS-terminating proxy.
The native QUIC address-discovery listener defaults to UDP :7842, matching the
client relay map. A proxy must preserve the public hostname and relay upgrade.

## Admission, limits and availability

Every connection proves possession of its Iroh endpoint key. That exact key
must have been registered with both Cloud device and endpoint signatures.
Admission requires an active host device or a controller with an active,
host-confirmed grant. Unknown, revoked and expired keys are denied. A short
directory publication lasts five minutes; relay authorization lasts eight
hours from publication. Reading it or sending traffic never renews that lease.

The service admits at most 256 connections, at most two per endpoint, and
limits inbound relay traffic to 64 KiB/s per connection. Database admission has
a two-second timeout and fails closed. Established links are rechecked every
20 seconds with at most 16 concurrent checks; known revocations, lease expiry
and database failures disconnect affected relay links. SIGINT/SIGTERM drains
the service. Application-level permissions and lease expiry are still checked
by the native host for every RPC.

An API-only outage does not affect established direct links. Relay operation
can continue while its database and service remain available. An unavailable
database stops relay admission and eventually existing relay links; an already
direct connection remains independent. Do not equate relay availability with
API availability or promise a new Cloud approval during an outage.

## Health and metrics

The HTTP listener exposes `/healthz`. This reports the relay listener, not a
successful Cloud approval or database query. Built-in Prometheus transport
metrics bind only to `127.0.0.1:9091` by default; change the port with
`PEER_RELAY_METRICS_BIND`. Scrape from the same network namespace. Metrics are
not published on the public relay port. Logs avoid credentials and RPC bodies.
Track accepts, disconnects, transport errors and relayed bytes alongside the
app's direct/relay route indicator. No traffic-reduction percentage is assumed.

## Verification

`cargo test --locked -p cloud-peer-relay` checks configuration. With a dedicated
`CLOUD_TEST_DATABASE_URL`, run:

```sh
cargo test --locked -p cloud-api v3_tests::peer_tests:: -- --ignored --test-threads=2
```

These tests exercise signed registration, CAS/replay protection, directory
isolation, relay lease expiry, exact grant confirmation, and an encrypted
relay-only connection through the production admission/monitor code. Test
certificates are trusted only inside the isolated fixture. The native app
separately tests direct connection survival after relay shutdown. Physical
two-PC and distinct-network acceptance remains a separate gate.

## Native host authorization projection

`GET /api/v3/remote/hosts/{id}/peer-authorizations` is owner-only and returns a
consistent snapshot of host, grant and device status with the registered
endpoint keys. Revoked records remain visible so the native host can fence
existing sessions. At most 256 grant records are supported; an over-limit
snapshot fails instead of returning a partial revocation list. Endpoint address
expiry does not erase the identity/authorization record.

Native hosts confirm exact approved revisions through signed HTTP, then project
them into the existing local collaboration permission engine. A Cloud API
failure does not renew local authorization. Local Owner changes/revocation take
precedence. Logout/account changes on the host suspend its Cloud projection.

Preset roles never infer native external effects or reusable automation grants.
For an explicit native policy, the owner updates a `custom` grant with
`allowedOperations` (for the existing WSS interface), `nativeClientKind`
(`desktop` or `agent`), and `nativeGrant` containing the native collaboration
grant. For example, this policy permits one agent to adjust only Alice's points:

```json
{
  "allowedOperations": [],
  "nativeClientKind": "agent",
  "nativeGrant": {
    "capabilities": ["points.read", "points.adjust", "commands.read"],
    "resources": {"viewer": ["alice"]},
    "maxPointsDelta": 5
  }
}
```

The normal expected-revision and host-confirmation gates still apply. The API
checks reserved field shapes and total policy size; the native host validates
the complete typed grant before confirming/importing it. Unknown capabilities,
effects or delegation policies are rejected by the host. No native session
becomes Owner through this projection. The complete native Cloud onboarding/UI
flow remains under implementation in the app repository.
