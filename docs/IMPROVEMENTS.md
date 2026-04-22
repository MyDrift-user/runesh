# RUNESH Library Improvement Plan

RUNESH crates are shared library building blocks, not standalone services.
Consumers assemble them into their own server and agent binaries. This plan
tracks stubs, insecure defaults, cross-platform gaps, and documented-but-not-
implemented behavior **inside the library code itself**. Out of scope: writing
server or agent binaries, or any pieces whose contract is "the caller provides
a real implementation" (for example coord server persistence, DNS server
listener, APNs / FCM push channels for MDM).

## Priority tiers

- **P0**: Insecure default that a consumer can silently wire in.
- **P1**: Function exists and compiles but returns empty / `Ok(())` / a stub,
  contradicting what the crate claims to do.
- **P2**: Cross-platform parity gap where one OS is real and others are thin.
- **P3**: Feature completeness, hardening, ergonomics.

## Issue register

### P0 - Insecure defaults

1. **`runesh-remote` default auth allows everything**
   `AllowAllAuth::authorize()` returns `Ok(())`. Rename to make the risk
   explicit, force consumers to opt in, and document the only acceptable
   production choice. `crates/runesh-remote/src/auth.rs:85-86`.

2. **`runesh-desktop` default auth allows everything**
   Same pattern. `crates/runesh-desktop/src/auth.rs:90`.

3. **`runesh-relay` defaults to `AuthMode::None`**
   Ships open. Make `None` require an explicit `::insecure_for_testing()`
   builder so it cannot be selected by default in consumer config.
   `crates/runesh-relay/src/server.rs:109-127`.

4. **`runesh-notify` SSRF allowlist is empty by default**
   `block_private_ips` is true but any public URL is accepted. Invert the
   default policy documentation and provide a `WebhookPolicy::strict()` helper
   that requires an explicit allowlist. `crates/runesh-notify/src/webhook.rs`.

5. **`runesh-vfs` Linux `allow_other` user-toggleable with warning only**
   Gate `allow_other` behind an explicit `unsafe_allow_any_uid` config flag,
   not a boolean that reads as harmless.
   `crates/runesh-vfs/src/platform/linux.rs:45-51`.

6. **`runesh-tun` wintun SHA256 pin is placeholder zeros**
   Production path must fail closed. Bake a real pin from a bundled
   `wintun.dll` artifact, and document how a consumer supplies their own.
   `crates/runesh-tun/src/tun_device.rs:55-108`.

7. **`runesh-desktop` has no secure-desktop check on Linux / macOS**
   Will capture a lock screen and inject into it. Add platform-appropriate
   checks (CGSessionCopyCurrentDictionary on macOS, loginctl / logind-dbus
   session lock on Linux) and return a typed "locked" error from the capturer.

### P1 - Stubs that contradict the crate's own claim

8. **`runesh-vfs` Windows Cloud Filter hydration is a placeholder**
   Registration happens but cfapi fetch / validate / dehydrate callbacks do
   not call `FileProvider`. Wire callbacks through the provider so placeholder
   files actually hydrate. `crates/runesh-vfs/src/platform/windows.rs:25-100`.

9. **`runesh-inventory` software enumeration returns empty**
   `collect_software_*` is referenced but not implemented on any platform.
   Implement: Windows registry `Uninstall` key + WinGet list, Linux dpkg / rpm
   / pacman, macOS `/Applications` + `system_profiler SPApplicationsDataType`.
   `crates/runesh-inventory/src/software.rs:1-30`.

10. **`runesh-desktop` clipboard `set()` is a noop**
    Returns `Ok(())` without calling the OS clipboard API. Implement
    OpenClipboard + SetClipboardData on Windows, NSPasteboard on macOS,
    XFixesSetSelectionOwner / wl-copy on Linux. `crates/runesh-desktop/src/clipboard.rs:87-89`.

11. **`runesh-baseline` enforcement mode is a no-op**
    `EnforcementMode::Enforce` is modeled and never acted upon. Add a
    `Remediator` trait per `StateDeclaration` kind (service start/stop, pkg
    install, file write, user/group create) with default implementations that
    call into `runesh-pkg` and platform process tools.
    `crates/runesh-baseline/src/checker.rs`.

12. **`runesh-baseline` Firewall and Custom declarations hardcoded to `Unknown`**
    Implement Windows Defender Firewall (`netsh advfirewall`), nftables /
    iptables, pf. For `Custom`, execute `check_command` with the same guardrails
    as `runesh-jobs`. `crates/runesh-baseline/src/checker.rs:256-272`.

13. **`runesh-patch` `affected_device_count()` is hardcoded 0**
    Accept a callable that returns the live count, or compute from the rollout
    plan input. `crates/runesh-patch/src/lib.rs:107`.

14. **`runesh-notify` has no email channel**
    The `#[cfg(feature = "email")]` path is empty. Implement SMTP via `lettre`
    with STARTTLS, DKIM-signed body optional, and connection pooling.
    `crates/runesh-notify/src/lib.rs`.

15. **`runesh-backup` retention policy never applied**
    `RetentionPolicy` is parsed and ignored. Implement the grandfather-
    father-son selector (keep last N daily / weekly / monthly / yearly) and
    run it during `gc()`. `crates/runesh-backup/src/store.rs:65`.

16. **`runesh-monitor` ICMP ping on Linux / macOS returns Unknown**
    Try unprivileged ICMP socket (`IPPROTO_ICMP` datagram) first, fall back
    to invoking `ping -c1 -W` as a subprocess. `crates/runesh-monitor/src/check.rs:285-306`.

17. **`runesh-stun` has no real STUN client**
    Add an async UDP probe with retransmit (RFC 8489 RTO sequence 500ms,
    1500ms, 3500ms), parse `XOR-MAPPED-ADDRESS`, classify NAT type from
    multi-server results. `crates/runesh-stun/src/client.rs`.

18. **`runesh-docker` CPU resize is ignored**
    `resize()` binds `_cpu`. Send `CpuQuota` and `CpuPeriod` in the update
    request. `crates/runesh-docker/src/lib.rs`.

19. **`runesh-hyperv` snapshot rename is a no-op**
    Use `Msvm_VirtualSystemSnapshotService.ModifySystemSettings` with a
    modified `ElementName` in the system settings data.
    `crates/runesh-hyperv/src/wmi_impl.rs:418-421`.

20. **`runesh-acl` parses `Proto` and SSH rules but never evaluates them**
    Wire protocol filtering into `AclEvaluator::try_evaluate`. Add a
    separate `SshAclEvaluator` with its own action enum
    (`accept | check | reject`). `crates/runesh-acl/src/eval.rs`.

### P2 - Cross-platform parity

21. **`runesh-desktop` X11 capture does not use XShm**
    Fall back to `XGetImage` is fine on first call, but for streaming, bind
    `XShmCreateImage` + `shmget` / `shmat`. Measure: should cut per-frame
    CPU cost roughly in half on X11.
    `crates/runesh-desktop/src/capture/x11.rs:85-87`.

22. **`runesh-desktop` Wayland cursor not rendered into frames**
    Wayland portal provides cursor metadata separately. Blend cursor bitmap
    into the captured frame in the encoder so viewers see a cursor.

23. **`runesh-vmware` snapshot / run_command / logs / resize all `NotSupported`**
    At least snapshot (`/api/vcenter/vm/{vm}/snapshots`) and resize
    (`hardware.cpu`, `hardware.memory` PATCH) are achievable. Guest run via
    `/api/vcenter/vm/{vm}/guest/processes` with VMware Tools.

### P3 - Hardening and completeness

24. **`runesh-auth` JWT validation pinned to HS256 only**
    Accept an `Algorithm` parameter; for RS256 / ES256 plumb a JWKS fetch and
    cache. `crates/runesh-auth/src/token.rs:165-176`.

25. **`runesh-vault` no rotation scheduler API**
    Add `rotate_master_key(new_key)` that re-wraps all entries, and a public
    `keys_expiring_within(Duration)` already present. No background job
    required inside the library. `crates/runesh-vault/src/lib.rs`.

26. **`runesh-relay` missing TLS wrapper and ACL-aware forwarding**
    Provide `RelayServer::with_tls(TlsAcceptor)`. Accept an `AclEvaluator`
    handle that is consulted before every `RecvPacket` send.
    `crates/runesh-relay/src/server.rs`.

27. **`runesh-appliance` OPNsense only**
    Document the trait as "bring your own driver", and provide one more
    reference implementation to validate the trait shape (UniFi Network
    controller REST API is the smallest additional impl).

28. **`runesh-jobs` in-memory queue only**
    Introduce a `TaskStore` trait and provide an in-memory impl and a
    sqlx-sqlite impl behind a feature flag. Workers then pull from the trait.
    `crates/runesh-jobs/src/executor.rs`.

29. **`runesh-asset`, `runesh-license`, `runesh-ipam` in-memory stores**
    Same pattern: introduce a `Store` trait, keep in-memory for tests, add
    a feature-gated sqlx-sqlite backend.

30. **`runesh-mdm` attestation verifiers missing**
    Out of scope for a library to deliver APNs / FCM push, but we can and
    should implement Android KeyAttestationStatement parsing (it is a well-
    defined X.509 extension OID 1.3.6.1.4.1.11129.2.1.17). Apple DeviceCheck
    requires a server round-trip to Apple's endpoint; provide the client.

## Out of scope for the library layer

Items that depend on consumer-provided infrastructure and should not be
implemented inside `crates/`:

- Coord HTTP server binary and node registry persistence.
- MagicDNS UDP / TCP :53 listener.
- TunnelManager to kernel WireGuard wiring.
- Persistence backends beyond a feature-flagged sqlite reference impl.
- MDM push channels (APNs, FCM, Windows MDM Enroll Service).
- Patch scheduler / worker loop.
- Appliance drivers beyond one reference impl.

These belong in consumer repos (RUMMZ, HARUMI, HARUMI-NET, the planned
Helvetia platform).

## Execution order

Work proceeds roughly P0 to P3. Where an item is small and contained
(for example default-auth renames), it is bundled with the next item in
the same crate to avoid many tiny PRs. Each item is its own PR against
a branch of the form `fix/<crate>-<short-name>` or
`feat/<crate>-<short-name>`, merged to main per the repo workflow rules.
