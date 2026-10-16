# Install, connect, operate

`lmd-top` runs on your workstation or an administration host. It uses your current
kubectl context and reads an existing Prometheus. Installing the client does not
install Kubernetes, llm-d, accelerator drivers, or exporters.

The `--init`, `--check-setup`, and `--version` commands below are **unreleased**;
use the current source checkout until a release containing them is published.

## Install

```bash
# Current checkout, including the new setup commands (Rust + C linker required):
bash install.sh --from-source --bin-dir "$HOME/.local/bin"
export PATH="$HOME/.local/bin:$PATH"

# Or install the latest published binary (no Rust needed):
curl -fsSL https://raw.githubusercontent.com/gspark-etri/lmd-top/main/install.sh | bash
```

The release installer requires curl, tar, install, and either sha256sum or shasum.
It requires a matching checksum before replacing the executable. Linux binaries
use glibc (x86_64 built on Ubuntu 22.04; aarch64 on Ubuntu 24.04); Alpine/musl is not
a prebuilt target. Build locally when your system has an older glibc.

## Connect once

```bash
kubectl config current-context
lmd-top --init
lmd-top --check-setup
lmd-top
```

`--init` asks for namespace, Prometheus host:port, and Grafana URL, then saves
`~/.config/lmd-top/lmd-top.yaml`. It preserves existing files, including custom
columns. Edit that file to change settings. For separate configurations:

```bash
LMD_CONFIG=./lab.yaml lmd-top --init
LMD_CONFIG=./lab.yaml lmd-top --check-setup
LMD_CONFIG=./lab.yaml lmd-top
```

Each command must use the same `LMD_CONFIG`. Environment variables `LMD_NS`,
`LMD_PROM`, and `LMD_GRAFANA` override the file. The file does **not** select a
kubectl context: use a dedicated `KUBECONFIG` when isolating clusters.

If Prometheus is only reachable inside the cluster, find its Service and run a
local port-forward in another terminal (replace the namespace/service/port):

```bash
kubectl get services -A
kubectl -n monitoring port-forward --address 127.0.0.1 svc/YOUR_PROMETHEUS_SERVICE 9090:9090
```

Enter `127.0.0.1:9090` in setup and keep that terminal running. The collector
currently supports plain HTTP host:port; it does not directly support TLS URLs,
URL path prefixes, or authentication headers. See the
[Kubernetes port-forward reference](https://kubernetes.io/docs/reference/kubectl/generated/kubectl_port-forward/).

`--check-setup` reuses the TUI's Setup checks. Exit 1 means required checks are
missing or unknown; exit 0 can still include warnings (for example missing Prometheus).
It checks platform prerequisites, not end-to-end inference success or complete
RBAC authorization. Open **6: Setup** for guided remediation; observe is the
default mode. Use `lmd-top --mode admin` when ready to review and apply changes.
Platform-specific PVC, gateway, driver and toolchain choices still need an operator.
If the Gateway is missing and several GatewayClasses are installed, Setup offers
one candidate row per class. Select the intended controller, review its generated
Gateway manifest, then apply. No class is preferred by vendor name. A single
installed class is offered directly for review. Class presence does not verify
controller readiness or inference-extension support.

## Update and recover

Re-run the installer with an explicit release version to upgrade or downgrade:

```bash
bash install.sh --version vX.Y.Z --bin-dir "$HOME/.local/bin"
```

Substitute a published tag. Keep a record of the prior tag before upgrading;
reinstall that tag to revert. Installation replaces the binary by a rename within
its destination directory, so interrupted downloads/checksum failures preserve
the previous binary. Config, audit, history, and cluster resources are not changed.
For old releases without `--version`, record the installer-selected tag.

For a bug report, include the version, OS/architecture, `--check-setup` output and
`--doctor` metric coverage. Review output for internal cluster/resource names
before sharing. No credentials or full kubeconfig are needed.
