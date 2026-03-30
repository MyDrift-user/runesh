# RUNESH Roadmap — Enterprise IT Management Suite

A Rust-native, cross-platform IT management platform covering the full device lifecycle: **deploy → inventory → manage → monitor → secure → recover**.

## Vision

Replace the patchwork of ConnectWise + Tailscale + CrowdStrike + Freshservice + Veeam + WDS/FOG with a single, unified, Rust-powered platform. Every component shares authentication, networking, and a common agent — no integration glue needed.

## Why Rust

| Advantage | Impact |
|-----------|--------|
| Single static binary agent (~5-10 MB, <20 MB RAM) | 10-50x lighter than .NET/Java agents |
| Memory safety for security-critical code | No buffer overflows in EDR (CrowdStrike 2024 lesson) |
| Zero-cost async I/O | Desktop streaming, backup chunking, file transfer at native speed |
| Type-safe state machines (enums + exhaustive match) | Impossible invalid ticket/alert/compliance transitions |
| Integrated WireGuard mesh (boringtun) | Every agent is automatically a secure mesh node |
| Cross-compile from one machine | Windows/Linux/macOS/ARM from a single CI pipeline |

## Standout Rust Ecosystem Crates

| Crate | What It Gives Us |
|-------|-----------------|
| `boringtun` | Cloudflare's WireGuard in userspace — battle-tested on millions of devices |
| `webauthn-rs` | Best WebAuthn/FIDO2 implementation in any language — passwordless auth |
| `yara-x` | Next-gen YARA from VirusTotal — threat detection rules |
| `tantivy` | Full-text search without ElasticSearch — knowledge base, log search |
| `opendal` | One API for every storage backend (S3, Azure, GCS, local, SFTP) |
| `rustic_core` | Restic-compatible backups — proven dedup/encryption at scale |
| `iroh` | NAT traversal baked into QUIC — by n0-computer |
| `fuser` | FUSE filesystem — virtual drives in file explorer |
| `quinn` | Production QUIC — multiplexed, encrypted agent-server comms |
| `fastcdc` | Content-defined chunking for deduplication |

---

## Current State (9 crates, shipped)

| Crate | Status | What It Does |
|-------|--------|-------------|
| `runesh-core` | ✅ Shipped | AppError, rate limiting, WS broadcast, file upload, middleware, metrics, service installer, graceful shutdown |
| `runesh-auth` | ✅ Shipped | OIDC + JWT + session management + RBAC + Axum middleware |
| `runesh-cli` | ✅ Shipped | Project scaffolding (init templates, compose, env) |
| `runesh-tauri` | ✅ Shipped | Tauri desktop helpers (tray, process mgmt, elevation) |
| `runesh-tun` | ✅ Shipped | Cross-platform TUN device (wintun + Linux) |
| `runesh-inventory` | ✅ Shipped | Hardware/software inventory (CPU, GPU, BIOS, battery, software, processes) |
| `runesh-remote` | ✅ Shipped | Remote file explorer + PTY terminal over WebSocket |
| `runesh-desktop` | ✅ Shipped | Remote desktop with multi-cursor, screen capture, input injection |
| `runesh-vfs` | ✅ Shipped | Virtual filesystem with cloud provider + overlay writes |
| `@runesh/ui` | ✅ Shipped | React/Next.js component library (sidebar, editor, data table, theme) |

---

## Roadmap

### Phase 1 — Foundation (Agent + Mesh)

The agent and mesh network are the backbone — everything else rides on them.

#### `runesh-agent` — Endpoint Daemon
The single binary that runs on every managed device. Composes inventory, remote, desktop, and monitoring into one persistent service.

- **Heartbeat loop** (configurable interval, default 60s) reporting to server
- **Task queue**: server pushes tasks (run script, collect inventory, install patch), agent executes
- **Secure enrollment**: agent generates keypair, sends CSR + org token, receives signed cert
- **Auto-update**: checks for new binary, downloads, replaces self, restarts service
- **Service installation**: registers as Windows service / systemd unit / launchd daemon (via `runesh-core::service`)
- **Feature composition**: inventory + remote + desktop + monitoring as feature-gated modules
- Key crates: `quinn` (QUIC transport), `rcgen` (certificate generation), `rustls` (mTLS)

#### `runesh-mesh` — WireGuard Mesh Network
Tailscale-like encrypted overlay network connecting all managed devices.

- **Coordination server** (Axum-based): stores node public keys, computes netmaps, distributes ACL policies
- **DERP relay server**: forwards encrypted packets when direct connection fails (~5-15% of cases)
- **NAT traversal**: STUN discovery + UDP hole punching (upgrades relay → direct, ~90% success)
- **MagicDNS**: `hostname.mesh.local` auto-resolves to mesh IP via `hickory-dns`
- **mDNS LAN discovery**: detect peers on same network, skip NAT traversal
- **ACL policies**: identity-based rules (who can reach what, on which ports)
- **WireGuard integration**: `boringtun` library + existing `runesh-tun` TUN device
- Build phases: tunnels → coordination → relay → NAT traversal → DNS → ACLs
- Key crates: `boringtun`, `quinn`, `iroh` (alternative), `hickory-dns`, `stun-client`, `mdns-sd`
- Reference projects: EasyTier (Rust P2P mesh), Firezone (zero-trust on boringtun), Headscale (open Tailscale coordination)

#### Extend `runesh-auth` — MFA + Directory Sync
- **TOTP MFA** via `totp-rs` (RFC 6238)
- **WebAuthn/FIDO2** via `webauthn-rs` — hardware key + biometric auth
- **LDAP/AD sync** via `ldap3` — import users/groups from Active Directory
- **SCIM provisioning** — auto-create/deactivate users from IdP
- **API key management** — scoped service-to-service tokens

**Milestone: MVP RMM** — agent + mesh gives you remote access to any device, anywhere, through encrypted tunnels.

---

### Phase 2 — Visibility (Monitoring + Notifications)

Can't manage what you can't see.

#### `runesh-monitor` — System Monitoring & Alerting
- **Check engine**: HTTP ping, TCP port, process running, disk threshold, custom script, Windows service status
- **Check scheduler**: cron-based intervals with jitter (prevent thundering herd)
- **Alert state machine**: `OK → Pending → Firing → Resolved` with flap prevention
- **Alert rules**: condition + severity + notification channel + escalation
- **Metric aggregation**: per-agent metrics rolled up for fleet dashboards
- **Time-series bridge**: expose Prometheus-compatible `/metrics` for Grafana, or embedded storage via `redb`
- Key crates: `cron`, `redb` (embedded KV), `hdrhistogram` (latency percentiles)

#### `runesh-notify` — Notification Dispatch
- **Channel trait**: `NotificationChannel { async fn send(&self, n: &Notification) }` with implementations for:
  - Webhook (arbitrary HTTP POST)
  - Email (SMTP via `lettre`)
  - Slack (`slack-morphism`)
  - Microsoft Teams (webhook)
  - Discord (webhook)
  - In-app push (via `runesh-core::ws_broadcast`)
- **Template engine**: format alerts into channel-specific messages via `tera`
- **Escalation chains**: if not acknowledged in X minutes, notify next person
- **User preferences**: per-user channel selection, quiet hours, dedup

**Milestone: Monitoring Platform** — see every device's health, get alerts when things break.

---

### Phase 3 — Security (EDR + Policy)

#### `runesh-edr` — Endpoint Detection & Response
- **Process monitoring**: track every process start/stop, parent-child trees, command lines
- **File integrity monitoring (FIM)**: baseline + continuous monitoring via `notify` + `sha2`
- **Network connection tracking**: which process connected where, on which port
- **YARA rule engine**: `yara-x` (VirusTotal's next-gen YARA in Rust) for behavioral detection
- **IoC matching**: hash sets, IP sets, domain sets with `aho-corasick` multi-pattern matching
- **Quarantine**: isolate suspicious files, optionally isolate endpoint from network (mesh ACL)
- **Vulnerability scanning**: match installed software versions against CVE databases
- Key crates: `yara-x`, `aho-corasick`, `maxminddb` (GeoIP), `notify`

#### `runesh-policy` — Configuration Management
- **Desired state declarations**: "this package must be installed", "this service must be running", "this registry key must exist"
- **Compliance checking**: agent evaluates policies on check-in, reports drift
- **Remediation**: auto-fix (install package, start service) or alert
- **Policy groups**: assign policies to device groups, user groups, or tags
- Key crates: `serde_yaml`, `petgraph` (DAG for dependency resolution)

**Milestone: Security Platform** — detect threats, enforce compliance, compete with CrowdStrike Falcon Go.

---

### Phase 4 — Service Desk (Tickets + Automation)

#### `runesh-itsm` — IT Service Management
- **Ticket lifecycle**: `Open → InProgress → Waiting → Resolved → Closed` as Rust enum state machine
- **SLA tracking**: response/resolution time targets with business hours, breach notifications
- **Knowledge base**: searchable articles via `tantivy` full-text search
- **Asset-ticket linking**: "this ticket is about this laptop" (links to `runesh-inventory`)
- **Auto-ticket creation**: monitoring alerts auto-create tickets
- **Customer portal**: end-users submit tickets, check status
- **Time tracking**: per-ticket work logging
- Key crates: `tantivy`, `chrono-tz` (business hours), `handlebars` (templates)

#### `runesh-automation` — Scripting & Runbooks
- **Script library**: centrally managed PowerShell/Bash/Python scripts with versioning
- **Execution engine**: spawn via PTY (`runesh-remote`), capture output, enforce timeout
- **Runbook engine**: multi-step DAG workflows with conditional branching (`petgraph`)
- **Scheduled tasks**: cron-based recurring automation
- **Desired-state engine**: declarative config that agents converge toward on each check-in
- Key crates: `petgraph`, `tera`, `cron`

**Milestone: Full ITSM** — tickets, automation, knowledge base. Compete with Freshservice/Jira SM.

---

### Phase 5 — Data Protection (Backup + Patch)

#### `runesh-backup` — Incremental Backup & Recovery
- **Content-defined chunking** (CDC) via `fastcdc` for dedup
- **Content-addressable storage**: chunks keyed by SHA-256, automatic dedup
- **Streaming pipeline**: `read → chunk → compress(zstd) → encrypt(chacha20) → upload`
- **Any storage backend** via `opendal` (S3, Azure Blob, GCS, local, SFTP — one API)
- **Restic compatibility** via `rustic_core` (read/write Restic repos)
- **Snapshot trees**: each backup is metadata pointing to chunks; incremental = only new chunks
- **Retention policies**: daily/weekly/monthly with configurable keep counts
- **Point-in-time restore**: browse any snapshot, restore individual files or full system
- **Browsable via VFS**: mount backup snapshots as virtual filesystems (`runesh-vfs`)
- Key crates: `fastcdc`, `opendal`, `rustic_core`, `chacha20poly1305`, `blake3`

#### `runesh-patch` — Patch Management
- **Windows**: Windows Update Agent API via WMI/COM for pending update detection + install
- **Linux**: apt/dnf/zypper/pacman abstraction for update checking
- **macOS**: `softwareupdate` CLI integration
- **Approval workflow**: scan → review → approve → schedule → deploy
- **Compliance reporting**: "95% of fleet is patched within SLA"
- Links to `runesh-inventory` (knows what software is installed)

**Milestone: Enterprise Suite** — backup + patch + everything before = ConnectWise/Datto tier.

---

### Phase 6 — Bare Metal (PXE + OS Deployment)

Full device lifecycle starts here — deploy an OS before you can manage it.

#### Separate Project: PXE Deploy System (consumes RUNESH)

PXE deployment is a **standalone project that consumes RUNESH crates**, not a crate within the monorepo. It has its own server processes (DHCP, TFTP, HTTP boot server), its own database, and its own web UI — too domain-specific to be a library crate.

**Why separate:**
- Runs its own network services (ProxyDHCP on port 67/4011, TFTP on port 69, HTTP boot server)
- Different dependency profile (raw sockets, boot file manipulation, WIM handling)
- Independent development lifecycle
- Is a *consumer* of RUNESH, not a *component* of it

**What it consumes from RUNESH:**
- `runesh-core` — AppError, WS broadcast (live deployment progress), DB pool
- `runesh-auth` — OIDC/JWT for the management web UI
- `runesh-inventory` — hardware info for driver matching
- `@runesh/ui` — Next.js frontend (sidebar, data-table, template editor)

**Project structure:**
```
runesh-deploy/                          # Separate repo, consumes RUNESH
├── crates/
│   ├── deploy-dhcp/                    # ProxyDHCP server (tokio + dhcproto)
│   ├── deploy-tftp/                    # Minimal TFTP for iPXE chainload
│   ├── deploy-http/                    # HTTP boot asset server (axum)
│   ├── deploy-ipxe/                    # iPXE script generation + menu builder
│   ├── deploy-engine/                  # Workflow/task engine (Tinkerbell-inspired)
│   ├── deploy-imaging/                 # WIM manipulation (wimlib FFI), multicast
│   └── deploy-api/                     # REST API + WebSocket (axum + runesh-core)
├── frontend/                           # Next.js + @runesh/ui
├── assets/
│   ├── ipxe/                           # Pre-built iPXE binaries
│   └── wimboot/                        # wimboot for WinPE HTTP boot
├── Cargo.toml
└── package.json                        # "@runesh/ui": "file:../RUNESH/packages/ui"
```

**PXE Boot Chain:**
```
Power on → DHCP (get IP) → ProxyDHCP (get boot server)
  → TFTP (load iPXE, ~100KB) → iPXE HTTP (boot menu + scripts)
  → OS installer (kernel+initrd or WinPE via wimboot)
  → Unattended install (autounattend.xml / preseed / kickstart / flake)
  → First boot → runesh-agent auto-enrolls → Fully managed
```

**Key components:**

- **ProxyDHCP server** — runs alongside existing DHCP (doesn't replace it). Uses `dhcproto` + tokio. Injects PXE boot options (DHCP options 66/67). Detects UEFI vs Legacy BIOS via client architecture field and serves the correct iPXE binary.

- **TFTP server** — only serves the initial ~100KB iPXE chainload binary. Once iPXE starts, everything switches to HTTP. Can use `tftpd` crate or custom (~200 lines, TFTP is a simple protocol).

- **HTTP boot server** — serves iPXE scripts, kernels, initrds, WinPE images, driver packs. HTTP is 10-100x faster than TFTP for large files. wimboot enables booting WinPE entirely over HTTP (no SMB/iSCSI needed).

- **iPXE script engine** — dynamically generates boot menus per-host based on MAC address, hardware profile, or asset tag. Like netboot.xyz but self-hosted with your own images. Custom iPXE binaries with embedded server URL.

- **Deployment workflow engine** — Tinkerbell-inspired task sequences: format disk → apply image → inject drivers → apply unattend → install apps → enroll agent. DAG-based execution with conditional steps.

**Supported OS deployments:**

| OS | Boot Method | Automation | Key Tech |
|----|-------------|-----------|----------|
| Windows 10/11 | WinPE via wimboot over HTTP | `autounattend.xml` | wimlib for WIM capture/apply/driver injection |
| Windows Server | WinPE via wimboot over HTTP | `autounattend.xml` + post-scripts | Same + domain join automation |
| Debian/Ubuntu | netboot kernel+initrd | `preseed.cfg` or autoinstall YAML | cloud-init for post-install |
| RHEL/Fedora | netboot kernel+initrd | `kickstart` (`inst.ks=http://...`) | Kickstart with `%post` scripts |
| Arch Linux | netboot kernel+initrd | `archinstall` JSON profiles | Profile served over HTTP |
| NixOS | netboot kernel+initrd | Flake configuration | `nixos-install --flake` pulls config from server |
| Custom NixOS | Custom ISO or netboot | Embedded flake in ISO | Pre-built with `nix-community/nixos-images` |
| Proxmox VE | netboot or ISO | TOML answer file (8.1+) | `proxmox-auto-install-assistant` |
| Any ISO | iPXE `sanboot` / memdisk | Manual or per-distro automation | For distros without netboot support |

**Windows-specific capabilities:**
- WIM image management via **wimlib** (cross-platform, faster than Microsoft's DISM, full compression support)
- Driver pack library organized by hardware model, auto-matched via SMBIOS info from `runesh-inventory`
- Task sequences: ordered multi-step deployment with conditional logic
- Multicast deployment: UDPcast for deploying to 50+ machines simultaneously on Gigabit Ethernet

**Linux-specific capabilities:**
- Kernel + initrd served over HTTP (each distro's netboot files)
- Template engine for preseed/kickstart/cloud-init with variables (hostname, IP, disk layout, packages)
- NixOS flake serving: deployment server hosts flake configs, nixos-install pulls them
- Arch archinstall JSON profiles served over HTTP
- Post-install hook: `curl | bash` to install `runesh-agent`

**Boot menu:**
- iPXE ANSI-color menu generated dynamically per-host
- Categories: Windows, Linux, Recovery Tools, Diagnostics
- Auto-selection based on MAC address or asset tag
- Timeout with default action (e.g., boot local disk after 10s if no selection)

**Key Rust crates:**
- `dhcproto` — DHCP packet parsing/construction
- `tftpd` — TFTP server
- `axum` — HTTP boot server + management API
- `tera` — template engine for unattend.xml/preseed/kickstart generation
- wimlib via FFI — WIM image manipulation

**Reference implementations to study:**
- **Tinkerbell** (Go, CNCF) — best modern architecture, composable workflow actions
- **netboot.xyz** — best iPXE script reference, 50+ distro support
- **FOG Project** (PHP) — multicast imaging, host registration
- **MAAS** (Python) — fastest OS installs, excellent API, auto-discovery
- **Foreman** (Ruby) — template-based provisioning, lifecycle management

**Integration with RUNESH suite:**
- `runesh-inventory` → hardware info for automatic driver matching
- `runesh-agent` → auto-installs as final deployment step
- `runesh-mesh` → auto-enrolls deployed machines into encrypted network
- `runesh-policy` → applies baseline compliance on first check-in
- `runesh-itsm` → links deployment to a provisioning ticket
- `runesh-monitor` → starts monitoring immediately after enrollment

---

### Phase 7 — Polish & Differentiation

#### `runesh-audit` — Centralized Audit & Compliance
- Tamper-evident log (hash chain / Merkle tree)
- Collects events from all crates
- Session recording playback (from `runesh-remote` CLI sessions)
- Compliance reports: SOC 2, ISO 27001, GDPR checklists
- Retention policies with archival

#### `runesh-gateway` — Reverse Proxy & Zero-Trust Access
- Expose internal services to the internet securely (Cloudflare Tunnel / ngrok equivalent)
- Identity-aware proxy: authenticate via `runesh-auth` before reaching the service
- Per-service ACLs tied to mesh identity
- Automatic TLS via Let's Encrypt (`rustls` + ACME client)
- Rate limiting per user/service (reuse `runesh-core`)

#### `runesh-dns` — Internal DNS & Service Discovery
- MagicDNS for mesh network (`hostname.mesh.local`)
- Split DNS: internal queries local, external forwarded
- DNS-over-HTTPS/TLS
- Service discovery records
- Built on `hickory-dns`

#### `runesh-mdm` — Mobile/Device Management
- Device enrollment (QR code / enrollment URL)
- Configuration profiles (Wi-Fi, VPN, email, certs)
- Remote lock/wipe
- App allowlisting/blocklisting
- Apple Business Manager + Android Enterprise integration
- Highest complexity — defer until core suite is solid

---

## Complete Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    CONTROL PLANE (Server)                        │
│                                                                  │
│  ┌──────────┐ ┌───────────┐ ┌──────────┐ ┌─────────────────┐   │
│  │ auth     │ │ core      │ │ mesh     │ │ deploy          │   │
│  │ OIDC+MFA │ │ API+WS    │ │ coord+   │ │ PXE+TFTP+HTTP   │   │
│  │ WebAuthn │ │ metrics   │ │ relay+   │ │ image repo      │   │
│  │ LDAP     │ │ rate lim  │ │ ACL+DNS  │ │ unattend/preseed│   │
│  └──────────┘ └───────────┘ └──────────┘ └─────────────────┘   │
│  ┌──────────┐ ┌───────────┐ ┌──────────┐ ┌─────────────────┐   │
│  │ monitor  │ │ itsm      │ │ backup   │ │ automation      │   │
│  │ checks   │ │ tickets   │ │ snapshots│ │ scripts+runbooks│   │
│  │ alerts   │ │ SLA+KB    │ │ retention│ │ desired-state   │   │
│  └──────────┘ └───────────┘ └──────────┘ └─────────────────┘   │
│  ┌──────────┐ ┌───────────┐ ┌──────────┐ ┌─────────────────┐   │
│  │ notify   │ │ policy    │ │ patch    │ │ audit           │   │
│  │ email    │ │ compliance│ │ OS+app   │ │ tamper-proof log│   │
│  │ slack    │ │ drift det.│ │ updates  │ │ session replay  │   │
│  └──────────┘ └───────────┘ └──────────┘ └─────────────────┘   │
│  ┌──────────┐ ┌───────────┐                                     │
│  │ gateway  │ │ dns       │                                     │
│  │ zero-    │ │ MagicDNS  │                                     │
│  │ trust    │ │ split DNS │                                     │
│  └──────────┘ └───────────┘                                     │
└────────────────────────┬────────────────────────────────────────┘
                         │ WireGuard mesh (encrypted)
┌────────────────────────┴────────────────────────────────────────┐
│                   AGENT (every managed device)                   │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │  runesh-agent daemon                                        │ │
│  │  heartbeat · task queue · auto-update · enrollment          │ │
│  ├────────────────────────────────────────────────────────────┤ │
│  │  inventory │ remote  │ desktop │ monitor │ edr │ backup    │ │
│  │  vfs       │ policy  │ patch   │ automation                │ │
│  └────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│              DASHBOARD (Next.js + @runesh/ui)                    │
│                                                                  │
│  Device fleet · Tickets · Network topology · Monitoring          │
│  Script editor · Backup browser · Deployment wizard              │
│  Asset CMDB · Compliance reports · Audit trail                   │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│              PXE BOOT (bare metal + VMs)                         │
│                                                                  │
│  ProxyDHCP → TFTP (iPXE chainload) → HTTP boot menu             │
│  → Windows (WinPE + WIM + autounattend.xml)                     │
│  → Linux (kernel + initrd + preseed/kickstart/flake)             │
│  → Recovery tools + diagnostics                                  │
│  → Post-install: agent enrollment → fully managed                │
└─────────────────────────────────────────────────────────────────┘
```

## Milestone Map

| Milestone | Crates / Projects | What You Can Compete With |
|-----------|-------------------|--------------------------|
| **M1: Remote Access** | core, auth, tun, inventory, remote, desktop, vfs | AnyDesk + basic RMM |
| **M2: Agent + Mesh** | + agent, mesh, auth extensions | Tailscale + RMM combo |
| **M3: Monitoring** | + monitor, notify | NinjaOne, Atera |
| **M4: Security** | + edr, policy | CrowdStrike Falcon Go |
| **M5: Service Desk** | + itsm, automation | Freshservice, Jira SM |
| **M6: Data Protection** | + backup, patch | ConnectWise, Datto |
| **M7: Bare Metal** | + runesh-deploy (separate project) | Baramundi, FOG, WDS/MDT, MAAS |
| **M8: Full Platform** | + audit, gateway, dns, mdm | ServiceNow + Tailscale + CrowdStrike |

## Project Count

| Category | Crates | Status |
|----------|--------|--------|
| RUNESH existing | 9 crates + UI package | ✅ Shipped |
| RUNESH Phase 1-2 | agent, mesh, monitor, notify | 🔜 Next |
| RUNESH Phase 3-5 | edr, policy, itsm, automation, backup, patch | 📋 Planned |
| RUNESH Phase 7 | audit, gateway, dns, mdm | 📋 Future |
| **RUNESH total** | **~22 crates + UI** | |
| Separate: runesh-deploy | 7 crates (dhcp, tftp, http, ipxe, engine, imaging, api) + frontend | 📋 Planned |
| **Grand total** | **~29 crates across 2 repos** | |

## Full Device Lifecycle

```
DEPLOY (PXE)  →  ENROLL (agent)  →  INVENTORY  →  CONFIGURE (policy)
     ↑                                                    ↓
REDEPLOY          ←  TICKET (itsm)  ←  PATCH  ←     MONITOR
                                                         ↓
                  BACKUP  ←  RESPOND (automation) ← DETECT (edr)
```

Every step is a RUNESH crate (or the separate deploy project). That's the vision.

## Related Projects

| Project | Repo | Relationship |
|---------|------|-------------|
| **RUNESH** | This repo | Shared crate library — the engine |
| **runesh-deploy** | Separate repo | PXE boot + OS deployment — consumes RUNESH crates |
| **RUMMZ** | Existing | Media management — consumer of RUNESH |
| **HARUMI** | Existing | Business suite — consumer of RUNESH |
| **HARUMI-NET** | Existing | WireGuard overlay network — consumer of RUNESH |
| **MoodleNG** | Existing | Learning management — consumer of RUNESH |
