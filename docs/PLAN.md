# RUNESH Implementation Plan

## Existing Crates (16 libraries + 1 CLI)

| Crate | Status | Gaps |
|-------|--------|------|
| `runesh-core` | Done | - |
| `runesh-auth` | Done | Missing: LDAP/AD adapter, SCIM adapter, SAML adapter |
| `runesh-cli` | Done | - |
| `runesh-tauri` | Done | - |
| `runesh-tun` | Done | macOS untested |
| `runesh-inventory` | Partial | Linux/macOS fact parity, per-OS management surfaces |
| `runesh-remote` | Done | Session recording |
| `runesh-desktop` | Broken | x11rb API on Linux, core-graphics API on macOS |
| `runesh-vfs` | Done | macOS CI needs macfuse |
| `runesh-telemetry` | Done | - |
| `runesh-acl` | Done | - |
| `runesh-mesh` | Types only | Needs boringtun tunnel I/O loop |
| `runesh-relay` | Types only | Needs integration testing |
| `runesh-coord` | Partial | Needs TS2021 WebSocket transport |
| `runesh-proxy` | Types only | Pingora wiring is consumer concern |
| `runesh-jobs` | Types only | Needs execution engine (spawn, capture, timeout) |

## New Crates Needed

### Tier 1: Complete the mesh + proxy (shippable product)

| Crate | What | Key deps |
|-------|------|----------|
| `runesh-dns` | MagicDNS, split DNS, service discovery | `hickory-dns` |
| `runesh-stun` | NAT traversal, STUN client, hole punching | `str0m` or custom |

### Tier 2: Monitoring + notifications (visibility)

| Crate | What | Key deps |
|-------|------|----------|
| `runesh-monitor` | Check engine (HTTP/TCP/process/disk), alert state machine, metric collection | `cron` |
| `runesh-notify` | Channel trait (email/Slack/Teams/webhook/Matrix/Discord/Ntfy), templates, escalation | `lettre`, `tera` |

### Tier 3: Security + compliance

| Crate | What | Key deps |
|-------|------|----------|
| `runesh-audit` | Append-only hash-chained event log, tamper detection | `sha2` |
| `runesh-vault` | Encrypted key-value store, rotation schedules, JIT decryption | `chacha20poly1305`, `aes-gcm` |
| `runesh-edr` | Process monitor, FIM, YARA rules, IoC matching, network tracking | `yara-x`, `notify`, `aho-corasick` |
| `runesh-baseline` | Desired-state declarations, drift detection, enforcement modes | `serde_yaml` |

### Tier 4: Infrastructure management

| Crate | What | Key deps |
|-------|------|----------|
| `runesh-appliance` | Uniform driver trait for network appliances (OPNsense/UniFi/FortiGate/MikroTik/Cisco/Sophos), config get/apply/rollback | `reqwest`, `russh`, `quick-xml`, `snmp2` |
| `runesh-ipam` | IP address management: prefixes, VLANs, IPs, allocation, utilization | - |
| `runesh-pkg` | Package manager trait (apt/dnf/pacman/winget/brew/pkg), install/remove/list/upgrade | - |

### Tier 5: Data + lifecycle

| Crate | What | Key deps |
|-------|------|----------|
| `runesh-backup` | Restic-compatible backup: chunking, encryption, retention, multi-backend storage | `rustic_core`, `opendal`, `fastcdc` |
| `runesh-asset` | Hardware/software asset tracking, warranty lookup, depreciation, lifecycle | - |
| `runesh-license` | License entitlements, assignments, utilization, renewal tracking | - |
| `runesh-patch` | Patch detection, ring-based rollout, CVE correlation, maintenance windows | - |

### Tier 6: Workloads + advanced

| Crate | What | Key deps |
|-------|------|----------|
| `runesh-workload` | Uniform VM/container/K8s driver trait (Docker/Podman/Hyper-V/KVM/Proxmox/K8s) | `bollard`, `kube`, `libvirt` |
| `runesh-flow` | Network flow collector (NetFlow/sFlow/IPFIX), bandwidth attribution | - |
| `runesh-mdm` | Device enrollment, config profiles, remote lock/wipe | - |

## Priority Order

1. Fix `runesh-desktop` (broken on Linux/macOS)
2. Add tunnel I/O to `runesh-mesh` (boringtun encapsulate/decapsulate loop)
3. Add TS2021 WebSocket to `runesh-coord`
4. Add execution engine to `runesh-jobs`
5. New: `runesh-dns` (MagicDNS)
6. New: `runesh-monitor` (check engine + alerts)
7. New: `runesh-notify` (dispatch channels)
8. New: `runesh-audit` (hash-chained log)
9. New: `runesh-vault` (encrypted secrets)
10. New: `runesh-pkg` (package manager trait)
11. New: `runesh-baseline` (drift detection)
12. New: `runesh-appliance` (network device drivers)
13. New: `runesh-backup` (restic-compatible)
14. New: `runesh-asset` (hardware lifecycle)
15. New: `runesh-license` (software licenses)
16. New: `runesh-edr` (endpoint detection)
17. New: `runesh-ipam` (IP management)
18. New: `runesh-workload` (VM/container drivers)
19. New: `runesh-patch` (patch management)
20. New: `runesh-flow` (network flow collector)
21. New: `runesh-stun` (NAT traversal)
22. New: `runesh-mdm` (mobile device management)

## Dependency Versions

Check docs.rs and crates.io before using any crate. Do not guess APIs from training data.

| Crate | Use for |
|-------|---------|
| `boringtun` 0.7 | WireGuard tunnel |
| `pingora` 0.8 | Reverse proxy (consumer binary) |
| `snow` 0.9 | Noise protocol |
| `quinn` 0.11 | QUIC transport |
| `hickory-dns` 0.25 | DNS server |
| `instant-acme` 0.7 | ACME certificates |
| `bollard` | Docker/Podman API |
| `kube` | Kubernetes API |
| `rustic_core` | Restic-compatible backup |
| `opendal` | Multi-backend storage |
| `yara-x` | YARA rule engine |
| `lettre` | SMTP email |
| `tera` | Templates |
| `tantivy` | Full-text search |
| `russh` | SSH client |
| `str0m` | WebRTC/ICE/STUN |
